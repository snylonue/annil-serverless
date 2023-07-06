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
use shuttle_persist::{Persist, PersistInstance};
use shuttle_secrets::{SecretStore, Secrets};
use tokio::{sync::RwLock, time::sleep};

use tower::ServiceBuilder;
use tower_http::cors::Any;

type Provider = OneDriveProvider;

#[derive(Debug, serde::Serialize)]
struct AnnilInfo {
    version: String,
    protocol_version: String,
    last_update: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct ClientInfoStorage {
    refresh_token: String,
    expire: u64,
    old_token: String,
}

impl ClientInfoStorage {
    fn from_client_info(info: &ClientInfo, expire: u64, old_token: String) -> Self {
        Self {
            refresh_token: info.refresh_token.clone(),
            expire,
            old_token,
        }
    }

    fn load(secret: &SecretStore, persist: &PersistInstance) -> Self {
        let refresh_token = secret.get("od_refresh_token").unwrap();
        match persist.load::<ClientInfoStorage>("refresh_token") {
            Ok(info) if info.old_token == refresh_token => info,
            _ => {
                log::warn!("failed to load refresh token or refresh token got updated, reading from secret store");
                Self {
                    refresh_token: refresh_token.clone(),
                    expire: 0,
                    old_token: refresh_token,
                }
            }
        }
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
async fn axum(
    #[Secrets] secret_store: SecretStore,
    #[Persist] persist: PersistInstance,
) -> shuttle_axum::ShuttleAxum {
    let token = ClientInfoStorage::load(&secret_store, &persist);

    let location = DriveLocation::from_id(DriveId(String::from(
        "b!uyGkzZXn6UeUrlI00cEEwB0U-PTBJVNIkX2vruaA2Wsnkoejm3etQpoha4pffHk9",
    )));
    let od = OneDriveClient::new(
        secret_store.get("od_client_id").unwrap(),
        ClientInfo::new(
            token.refresh_token,
            secret_store.get("od_client_secret").unwrap(),
            location,
        ),
    )
    .await
    .unwrap();

    let provider = Arc::new(AnnilProvider::new(Provider::new(od).await.unwrap()));

    let pd = Arc::clone(&provider);
    tokio::spawn(async move {
        loop {
            let p = pd.read().await;
            if p.drive.is_expired() {
                log::debug!("token expired, refreshing");
                match p.drive.refresh().await {
                    Ok(_) => log::debug!("new token will expire at {}", p.drive.expire()),
                    Err(e) => log::error!("refresh failed: {e}"),
                };
                match persist.save(
                    "refresh_token",
                    ClientInfoStorage::from_client_info(
                        &*p.drive.client_info().await,
                        p.drive.expire(),
                        token.old_token.clone(),
                    ),
                ) {
                    Ok(_) => {}
                    Err(e) => log::error!("persist error: {e}"),
                };
            }
            sleep(Duration::from_secs(
                p.drive.expire().checked_sub(now().as_secs()).unwrap_or(10),
            ))
            .await
        }
    });

    let annil_state = Arc::new(AnnilState {
        version: String::from("AnnilServerless v0.4.0"),
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
        .route("/:album_id/cover", get(cover_raw))
        .route("/:album_id/:disc_id/cover", get(cover_raw))
        .route(
            "/:album_id/:disc_id/:track_id",
            get(aduio_raw).head(annil::route::user::audio_head::<Provider>),
        )
        .route(
            "/admin/reload",
            post(annil::route::admin::reload::<Provider>),
        )
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
