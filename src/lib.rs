use std::{
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anni_provider::{
    providers::{
        drive::{DriveAuth, DriveProviderSettings},
        DriveProvider,
    },
    AnniProvider, ProviderError,
};
use axum::{http::StatusCode, response::IntoResponse, routing::get, Extension, Json, Router};
use sync_wrapper::SyncWrapper;

#[derive(Debug, serde::Serialize)]
struct AnnilInfo {
    version: String,
    protocol_version: String,
    last_update: u64,
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

async fn info(Extension(state): Extension<Arc<State>>) -> Json<AnnilInfo> {
    // returns a last_update that is hard-encoded
    Json(AnnilInfo {
        version: String::from("AnnilServerless v0.1.0"),
        protocol_version: String::from("0.4.1"),
        last_update: state.last_update,
    })
}

async fn albums(Extension(state): Extension<Arc<State>>) -> Result<Json<Vec<String>>, AnniError> {
    let alb = state.provider.albums().await?;
    Ok(Json(alb.into_iter().map(|s| s.to_string()).collect()))
}

#[shuttle_service::main]
async fn axum() -> shuttle_service::ShuttleAxum {
    let state = Arc::new(State {
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
                token_path: PathBuf::from_str(r"D:\files\code\annil_serverless\token").unwrap(),
            },
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
        .layer(Extension(state));
    let sync_wrapper = SyncWrapper::new(router);

    Ok(sync_wrapper)
}
