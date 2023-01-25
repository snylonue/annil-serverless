use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anni_provider::ProviderError;
use anni_provider_od::{
    onedrive_api::{DriveId, DriveLocation},
    OneDriveClient, OneDriveProvider,
};
use annil::{
    provider::AnnilProvider,
    state::{AnnilKeys, AnnilState},
};
use axum::{
    http::{Method, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Router,
};
use jwt_simple::prelude::HS256Key;
use shuttle_secrets::SecretStore;
use sync_wrapper::SyncWrapper;
use tokio::sync::RwLock;

use tower::ServiceBuilder;
use tower_http::cors::Any;

type Provider = OneDriveProvider;

#[derive(Debug, serde::Serialize)]
struct AnnilInfo {
    version: String,
    protocol_version: String,
    last_update: u64,
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

#[shuttle_service::main]
async fn axum(
    #[shuttle_secrets::Secrets] secret_store: SecretStore,
) -> shuttle_service::ShuttleAxum {
    let location = DriveLocation::from_id(DriveId(String::from(
        "b!uyGkzZXn6UeUrlI00cEEwB0U-PTBJVNIkX2vruaA2Wsnkoejm3etQpoha4pffHk9",
    )));
    let od = OneDriveClient::new(
        secret_store.get("od_refresh_token").unwrap(),
        secret_store.get("od_client_id").unwrap(),
        secret_store.get("od_client_secret").unwrap(),
        location,
    )
    .await
    .unwrap();

    let provider = Arc::new(AnnilProvider::new(Provider::new(od).await.unwrap()));

    let annil_state = Arc::new(AnnilState {
        version: String::from("AnnilServerless v0.3.0"),
        last_update: RwLock::new(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        ),
        etag: RwLock::new(provider.compute_etag().await.unwrap()),
        metadata: None,
    });

    let key = Arc::new(AnnilKeys {
        sign_key: HS256Key::from_bytes(secret_store.get("sign_key").unwrap().as_bytes()),
        admin_token: secret_store.get("admin_token").unwrap(),
        share_key: HS256Key::from_bytes(secret_store.get("share_key").unwrap().as_bytes())
            .with_key_id(&secret_store.get("share_key_id").unwrap()),
    });

    let router = Router::new()
        .route("/info", get(annil::route::user::info))
        .route("/albums", get(annil::route::user::albums::<Provider>))
        .route(
            "/:album_id/cover",
            get(annil::route::user::cover::<Provider>),
        )
        .route(
            "/:album_id/:disc_id/cover",
            get(annil::route::user::cover::<Provider>),
        )
        .route(
            "/:album_id/:disc_id/:track_id",
            get(annil::route::user::audio::<Provider>)
                .head(annil::route::user::audio_head::<Provider>),
        )
        .route(
            "/admin/reload",
            post(annil::route::admin::reload::<Provider>),
        )
        // .route("/admin/update_token", get(update_token))
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
        );
    let sync_wrapper = SyncWrapper::new(router);

    Ok(sync_wrapper)
}
