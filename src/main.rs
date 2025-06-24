use std::{
    num::NonZeroU8,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anni_provider::{AnniProvider, ProviderError};
use anni_provider_od::{
    onedrive_api::{DriveId, DriveLocation},
    ClientInfo, OneDriveClient, OneDriveProvider,
};
use annil::{
    extractor::track::TrackIdentifier,
    provider::AnnilProvider,
    state::{AnnilKeys, AnnilState},
};
use axum::{
    extract::Path,
    http::{
        header::{ACCESS_CONTROL_EXPOSE_HEADERS, CACHE_CONTROL},
        Method, StatusCode,
    },
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Extension, Router,
};
use jwt_simple::prelude::HS256Key;
use serde::Deserialize;
use shuttle_runtime::{SecretStore, Secrets};
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::cors::Any;

// #[derive(Debug, serde::Serialize)]
// struct AnnilInfo {
//     version: String,
//     protocol_version: String,
//     last_update: u64,
// }

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
            Self::AnniError(error) => (
                StatusCode::NOT_FOUND,
                [(CACHE_CONTROL, "private")],
                error.to_string(),
            ),
        }
        .into_response()
    }
}

async fn aduio_raw(
    track: TrackIdentifier,
    Extension(provider): Extension<Arc<AnnilProvider<OneDriveProvider>>>,
) -> Response {
    let provider = provider.read().await;

    let uri = match provider
        .audio_url(&track.album_id.to_string(), track.disc_id, track.track_id)
        .await
    {
        Ok((uri, _)) => uri,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(CACHE_CONTROL, "private")],
                format!("{e:?}"),
            )
                .into_response()
        }
    };

    let info = match provider
        .get_audio_info(&track.album_id.to_string(), track.disc_id, track.track_id)
        .await
    {
        Ok(info) => info,
        Err(e) => return Error::from(e).into_response(),
    };
    let header = [(
        ACCESS_CONTROL_EXPOSE_HEADERS,
        "X-Origin-Type, X-Origin-Size, X-Duration-Seconds, X-Audio-Quality".to_string(),
    )];
    let headers = [
        ("X-Origin-Type", format!("audio/{}", info.extension)),
        ("X-Origin-Size", format!("{}", info.size)),
        ("X-Duration-Seconds", format!("{}", info.duration)),
        ("X-Audio-Quality", String::from("lossless")),
    ];

    (header, headers, Redirect::temporary(&uri)).into_response()
}

#[derive(Deserialize)]
struct CoverPath {
    album_id: String,
    disc_id: Option<NonZeroU8>,
}

async fn cover_raw(
    Path(CoverPath { album_id, disc_id }): Path<CoverPath>,
    Extension(provider): Extension<Arc<AnnilProvider<OneDriveProvider>>>,
) -> Response {
    let provider = provider.read().await;

    let uri = match provider.cover_url(&album_id, disc_id).await {
        Ok(uri) => uri,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(CACHE_CONTROL, "private")],
                format!("{e:?}"),
            )
                .into_response()
        }
    };

    Redirect::temporary(&uri).into_response()
}

fn now() -> Duration {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
}

#[shuttle_runtime::main]
async fn axum(#[Secrets] secret_store: SecretStore) -> shuttle_axum::ShuttleAxum {
    let location = DriveLocation::from_id(DriveId(String::from(
        "b!uyGkzZXn6UeUrlI00cEEwB0U-PTBJVNIkX2vruaA2Wsnkoejm3etQpoha4pffHk9",
    )));
    let od = OneDriveClient::new(
        secret_store.get("od_client_id").unwrap(),
        ClientInfo::new(
            secret_store.get("od_refresh_token").unwrap(),
            secret_store.get("od_client_secret").unwrap(),
            location,
        ),
    )
    .await
    .unwrap();

    let od = Arc::new(od);

    let provider = Arc::new(AnnilProvider::new(
        OneDriveProvider::new(Arc::clone(&od), "/anni-ws".to_owned(), 0)
            .await
            .unwrap(),
    ));

    // let pd = Arc::clone(&provider);

    let annil_state = Arc::new(AnnilState {
        version: String::from(concat!("AnnilServerless v", env!("CARGO_PKG_VERSION"))),
        last_update: RwLock::new(now().as_secs()),
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
        .route(
            "/albums",
            get(annil::route::user::albums::<OneDriveProvider>),
        )
        .route("/:album_id/cover", get(cover_raw))
        .route("/:album_id/:disc_id/cover", get(cover_raw))
        .route(
            "/:album_id/:disc_id/:track_id",
            get(aduio_raw).head(annil::route::user::audio_head::<OneDriveProvider>),
        )
        .route(
            "/admin/reload",
            post(annil::route::admin::reload::<OneDriveProvider>),
        )
        .route("/admin/sign", post(annil::route::admin::sign))
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

    Ok(router.into())
}
