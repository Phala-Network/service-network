use crate::{WorkerRuntimeStatus, CONFIG, REQ_CLIENT, WR};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Extension, Router};
use bytes::Bytes;
use futures::future::try_join_all;
use hyper::Server;
use log::{info, warn};
use reqwest::header::CONTENT_TYPE;
use reqwest::StatusCode;
use service_network::utils::CONTENT_TYPE_BIN;
use std::sync::Arc;
use tokio::task::JoinHandle;

pub struct Shared {}

pub async fn start_public_forwarder() -> () {
    let binds = &CONFIG.local_worker().forwarder_bind_addresses;
    let binds = binds
        .iter()
        .map(|addr| tokio::spawn(start_server(addr.to_string())));
    let binds: Vec<JoinHandle<()>> = binds.collect();
    let result = try_join_all(binds).await;
    if result.is_err() {
        panic!("start_public_forwarder: {}", result.err().unwrap());
    }
}

async fn start_server(addr: String) {
    info!("Starting public forwarder on {}.", &addr);

    let shared = Shared {};

    let addr = &addr.parse().unwrap();
    let router = create_router();

    let router = router.layer(Extension(Arc::new(shared)));

    Server::bind(addr)
        .serve(router.into_make_service())
        .await
        .unwrap();
}

fn create_router() -> Router {
    let router = Router::new();

    let router = router.route("/", post(forwarder));

    router
}

async fn forwarder(body: Bytes) -> impl IntoResponse {
    let wr = WR.read().await;
    let status = wr.status.clone();
    drop(wr);

    match status {
        WorkerRuntimeStatus::Started => {
            let wr = WR.read().await;
            let url = &wr.pruntime_rpc_prefix.clone();
            drop(wr);
            let req = REQ_CLIENT
                .post(url)
                .header(CONTENT_TYPE, CONTENT_TYPE_BIN)
                .body(body)
                .send()
                .await;
            match req {
                Ok(res) => {
                    let status = res.status() as StatusCode;
                    let ret_bytes = res.bytes().await.unwrap_or_default();
                    (status, ret_bytes)
                }
                Err(err) => {
                    warn!("Failed to forward request to pRuntime: {:?}", err);
                    (StatusCode::BAD_GATEWAY, Bytes::new())
                }
            }
        }
        _ => (StatusCode::BAD_GATEWAY, Bytes::new()),
    }
}
