use std::{
    collections::HashMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anni_google_drive3::oauth2::{self, storage::TokenInfo};
use anni_provider::{
    providers::{
        drive::{self, DriveAuth, DriveProviderSettings},
        DriveProvider, MultipleProviders,
    },
    AnniProvider, ProviderError,
};
use annil::{
    provider::AnnilProvider,
    state::{AnnilKeys, AnnilState},
};
use axum::{
    async_trait,
    extract::Query,
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Router,
};
use jwt_simple::prelude::HS256Key;
use shuttle_persist::PersistInstance;
use shuttle_secrets::SecretStore;
use sync_wrapper::SyncWrapper;
use tokio::sync::RwLock;

use tower::ServiceBuilder;
use tower_http::cors::Any;

type Provider = MultipleProviders;

#[derive(Debug, serde::Serialize)]
struct AnnilInfo {
    version: String,
    protocol_version: String,
    last_update: u64,
}

struct TokenStorage {
    persist: Arc<PersistInstance>,
}

#[async_trait]
impl oauth2::storage::TokenStorage for TokenStorage {
    async fn set(&self, _: &[&str], token: oauth2::storage::TokenInfo) -> anyhow::Result<()> {
        Ok(self.persist.save("token", token)?)
    }

    /// Retrieve a token stored by set for the given set of scopes
    async fn get(&self, _: &[&str]) -> Option<oauth2::storage::TokenInfo> {
        self.persist.load("token").ok()
    }
}

#[derive(Debug)]
enum Error {
    AnniError(ProviderError),
}

impl From<ProviderError> for Error {
    fn from(error: ProviderError) -> Self {
        Self::AnniError(error)
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::AnniError(error) => (StatusCode::NOT_FOUND, error.to_string()),
        }
        .into_response()
    }
}

async fn drive_provider(
    persist: Arc<PersistInstance>,
) -> Result<Box<dyn AnniProvider + Send + Sync>, ProviderError> {
    DriveProvider::new(
        DriveAuth::InstalledFlow {
            client_id: String::from(
                "453004067441-3vj45hga37etmmuhjplucfeqgehu7a93.apps.googleusercontent.com",
            ),
            client_secret: String::from("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"),
            project_id: Some(String::from("annil_serverless")),
        },
        DriveProviderSettings {
            corpora: String::from("user"),
            drive_id: None,
        },
        None,
        drive::TokenStorage::Custom(Box::new(TokenStorage {
            persist: persist.clone(),
        })),
    )
    .await
    .map::<Box<dyn AnniProvider + Send + Sync>, _>(|p| Box::new(p))
}

async fn update_token(
    Extension(persist): Extension<Arc<PersistInstance>>,
    Extension(provider): Extension<Arc<AnnilProvider<Provider>>>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<&'static str, Error> {
    let token: TokenInfo = serde_json::from_str(q.get("token").unwrap()).unwrap();
    persist.save("token", token).unwrap();
    let new_provider = drive_provider(persist).await?;
    let mut p = provider.write().await;
    *p = MultipleProviders::new(vec![new_provider]);
    Ok("token updated")
}

#[shuttle_service::main]
async fn axum(
    #[shuttle_persist::Persist] persist: PersistInstance,
    #[shuttle_secrets::Secrets] secret_store: SecretStore,
) -> shuttle_service::ShuttleAxum {
    let initial_token: Option<TokenInfo> = secret_store
        .get("token")
        .map(|token| serde_json::from_str(&token).unwrap());
    match persist.load::<oauth2::storage::TokenInfo>("token") {
        Ok(token) => match initial_token {
            Some(init) if init.expires_at > token.expires_at => {
                persist.save("token", init).unwrap()
            }
            _ => {}
        },
        Err(_) => {
            persist
                .save(
                    "token",
                    initial_token.expect("failed to load initial token"),
                )
                .unwrap();
        }
    }
    let persist = Arc::new(persist);

    let provider = Arc::new(AnnilProvider::new(Provider::new(
        drive_provider(Arc::clone(&persist))
            .await
            .into_iter()
            .collect(),
    )));

    let annil_state = Arc::new(AnnilState {
        version: String::from("AnnilServerless v0.2.1"),
        last_update: RwLock::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        ),
        etag: RwLock::new(provider.compute_etag().await.unwrap()),
    });

    let key = Arc::new(AnnilKeys {
        sign_key: HS256Key::from_bytes(secret_store.get("sign_key").unwrap().as_bytes()),
        admin_token: secret_store.get("admin_token").unwrap(),
        share_key: HS256Key::from_bytes(secret_store.get("share_key").unwrap().as_bytes()),
    });

    let router = Router::new()
        .route("/info", get(annil::route::user::info))
        .route("/albums", get(annil::route::user::albums::<Provider>))
        .route("/:album/cover", get(annil::route::user::cover::<Provider>))
        .route(
            "/:album/:disc/cover",
            get(annil::route::user::cover::<Provider>),
        )
        .route(
            "/:album/:disc/:track",
            get(annil::route::user::audio::<Provider>)
                .head(annil::route::user::audio_head::<Provider>),
        )
        .route(
            "/admin/reload",
            post(annil::route::admin::reload::<Provider>),
        )
        .route("/admin/update_token", get(update_token))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_methods([Method::GET, Method::OPTIONS, Method::POST])
                .allow_headers(Any)
                .allow_origin(Any),
        )
        .layer(
            ServiceBuilder::new()
                .layer(Extension(Arc::clone(&annil_state)))
                .layer(Extension(Arc::clone(&provider)))
                .layer(Extension(Arc::clone(&key))),
        )
        .layer(Extension(persist));
    let sync_wrapper = SyncWrapper::new(router);

    Ok(sync_wrapper)
}
