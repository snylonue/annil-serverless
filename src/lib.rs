pub use anni_provider_drive_token_storage as anni_provider;

use std::{
    num::NonZeroU8,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anni_provider::{
    providers::{
        drive::{
            oauth2::{self, storage::TokenInfo},
            DriveAuth, DriveProviderSettings,
        },
        DriveProvider,
    },
    AnniProvider, ProviderError, Range,
};
use axum::{
    async_trait,
    body::{Empty, StreamBody},
    extract::Path,
    http::{
        header::{ACCESS_CONTROL_EXPOSE_HEADERS, CONTENT_LENGTH, CONTENT_TYPE},
        Method, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use shuttle_persist::PersistInstance;
use shuttle_secrets::SecretStore;
use sync_wrapper::SyncWrapper;
use tokio::sync::RwLock;
use tokio_util::io::ReaderStream;
use tower_http::cors::Any;

#[derive(Debug, serde::Serialize)]
struct AnnilInfo {
    version: String,
    protocol_version: String,
    last_update: u64,
}

struct TokenStorage {
    persist: PersistInstance,
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

struct State {
    provider: DriveProvider,
    last_update: u64,
}

#[derive(Debug)]
struct AnniError {
    error: ProviderError,
}

impl From<ProviderError> for AnniError {
    fn from(error: ProviderError) -> Self {
        Self { error }
    }
}

impl IntoResponse for AnniError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, self.error.to_string()).into_response()
    }
}

async fn info(Extension(state): Extension<Arc<RwLock<State>>>) -> Json<AnnilInfo> {
    Json(AnnilInfo {
        version: String::from("AnnilServerless v0.1.0"),
        protocol_version: String::from("0.4.1"),
        last_update: state.read().await.last_update,
    })
}

async fn albums(
    Extension(state): Extension<Arc<RwLock<State>>>,
) -> Result<Json<Vec<String>>, AnniError> {
    let s = state.read().await;
    let alb = s.provider.albums().await?;
    Ok(Json(alb.into_iter().map(|s| s.to_string()).collect()))
}

async fn audio(
    Extension(state): Extension<Arc<RwLock<State>>>,
    Path((album_id, disc_id, track_id)): Path<(String, NonZeroU8, NonZeroU8)>,
) -> Result<impl IntoResponse, AnniError> {
    let s = state.read().await;
    let audio = s
        .provider
        .get_audio(&album_id, disc_id, track_id, Range::FULL)
        .await?;
    Ok(StreamBody::new(ReaderStream::new(audio.reader)))
}

async fn audio_head(
    Extension(state): Extension<Arc<RwLock<State>>>,
    Path((album_id, disc_id, track_id)): Path<(String, NonZeroU8, NonZeroU8)>,
) -> Result<impl IntoResponse, AnniError> {
    let s = state.read().await;
    let info = s
        .provider
        .get_audio_info(&album_id, disc_id, track_id)
        .await?;
    let response = Response::builder()
        .status(200)
        .header("X-Origin-Type", format!("audio/{}", info.extension))
        .header("X-Origin-Size", info.size)
        .header("X-Audio-Quality", "lossless")
        .header(
            ACCESS_CONTROL_EXPOSE_HEADERS,
            "X-Origin-Type, X-Origin-Size, X-Duration-Seconds, X-Audio-Quality",
        )
        .header(CONTENT_LENGTH, info.size)
        .header(CONTENT_TYPE, format!("audio/{}", info.extension))
        .body(Empty::new())
        .unwrap();
    Ok(response)
}

async fn cover(
    Extension(state): Extension<Arc<RwLock<State>>>,
    Path((album_id, disc_id)): Path<(String, Option<NonZeroU8>)>,
) -> Result<impl IntoResponse, AnniError> {
    let s = state.read().await;
    let cover = s.provider.get_cover(&album_id, disc_id).await?;
    Ok(StreamBody::new(ReaderStream::new(cover)))
}

async fn reload(Extension(state): Extension<Arc<RwLock<State>>>) -> Result<(), AnniError> {
    let mut s = state.write().await;
    s.provider.reload().await?;
    s.last_update = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    Ok(())
}

#[shuttle_service::main]
async fn axum(
    #[shuttle_persist::Persist] persist: PersistInstance,
    #[shuttle_secrets::Secrets] secret_store: SecretStore,
) -> shuttle_service::ShuttleAxum {
    let storage = TokenStorage { persist };
    let initial_token: Option<TokenInfo> = secret_store
        .get("token")
        .map(|token| serde_json::from_str(&token).unwrap());
    match storage.persist.load::<oauth2::storage::TokenInfo>("token") {
        Ok(token) => match initial_token {
            Some(init) if init.expires_at > token.expires_at => {
                storage.persist.save("token", init).unwrap()
            }
            _ => {}
        },
        Err(_) => {
            storage
                .persist
                .save(
                    "token",
                    initial_token.expect("failed to load initial token"),
                )
                .unwrap();
        }
    }
    let state = RwLock::new(State {
        provider: DriveProvider::new(
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
                token_path: PathBuf::from_str(r"./token").unwrap(),
            },
            Box::new(storage),
            None,
        )
        .await
        .unwrap(),
        last_update: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    });

    let router = Router::new()
        .route("/info", get(info))
        .route("/albums", get(albums))
        .route(
            "/:album/cover",
            get(|extension, Path(album_id): Path<String>| async {
                cover(extension, Path((album_id, NonZeroU8::new(1)))).await
            }),
        )
        .route("/:album/:disc/cover", get(cover))
        .route("/:album/:disc/:track", get(audio).head(audio_head))
        .route("/admin/reload", post(reload))
        .layer(
            tower_http::cors::CorsLayer::new()
                .allow_methods([Method::GET, Method::OPTIONS])
                .allow_headers(Any)
                .allow_origin(Any),
        )
        .layer(Extension(Arc::new(state)));
    let sync_wrapper = SyncWrapper::new(router);

    Ok(sync_wrapper)
}
