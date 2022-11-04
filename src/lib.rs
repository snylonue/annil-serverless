use std::time::{SystemTime, UNIX_EPOCH};

use axum::{routing::get, Router, Json};
use sync_wrapper::SyncWrapper;

#[derive(Debug, serde::Serialize)]
struct AnnilInfo {
    version: String,
    protocol_version: String,
    last_update: u64
}

async fn info() -> Json<AnnilInfo> {
    // returns a last_update that is hard-encoded
    Json(AnnilInfo {
        version: String::from("AnnilServerless v0.1.0"),
        protocol_version: String::from("0.4.1"),
        last_update: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
    })
}

#[shuttle_service::main]
async fn axum() -> shuttle_service::ShuttleAxum {
    let router = Router::new().route("/info", get(info));
    let sync_wrapper = SyncWrapper::new(router);

    Ok(sync_wrapper)
}