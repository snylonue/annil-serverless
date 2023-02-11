use std::{sync::Arc, time::{SystemTime, UNIX_EPOCH}, env::var};
use anni_provider_od::{onedrive_api::{DriveLocation, DriveId}, OneDriveClient, OneDriveProvider};
use annil::{provider::AnnilProvider, state::{AnnilState, AnnilKeys}};
use axum::{Router, routing::{get, post}, http::Method, Extension, Server};
use jwt_simple::prelude::HS256Key;
use tokio::sync::RwLock;
use tower::ServiceBuilder;
use tower_http::cors::Any;

type Provider = OneDriveProvider;

#[tokio::main]
async fn main() {
    let location = DriveLocation::from_id(DriveId(String::from(
        "b!uyGkzZXn6UeUrlI00cEEwB0U-PTBJVNIkX2vruaA2Wsnkoejm3etQpoha4pffHk9",
    )));
    let od = OneDriveClient::new(
        dbg!(String::from(var("od_refresh_token").unwrap())),
        dbg!(String::from(var("od_client_id").unwrap())),
        dbg!(String::from(var("od_client_secret").unwrap())),
        location,
    )
    .await
    .unwrap();

    let provider = Arc::new(AnnilProvider::new(Provider::new(od).await.unwrap()));

    let annil_state = Arc::new(AnnilState {
        version: String::from("AnnilServerless v0.3.3"),
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
        sign_key: HS256Key::from_bytes(var("sign_key").unwrap().as_bytes()),
        admin_token: String::from(var("admin_token").unwrap()),
        share_key: HS256Key::from_bytes(var("share_key").unwrap().as_bytes())
            .with_key_id(&var("share_key_id").unwrap()),
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

    Server::bind(&"127.0.0.1:8000".parse().unwrap())
        .serve(router.into_make_service())
        .await
        .unwrap()
}