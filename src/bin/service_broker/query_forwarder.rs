use crate::local_worker::KnownLocalWorkerStatus;
use crate::{CONFIG, LW_MAP, REQ_CLIENT};
use axum::extract::Path;
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Extension, Router};
use bytes::Bytes;
use futures::future::try_join_all;
use hyper::Server;
use log::{debug, info, warn};
use service_network::utils::CONTENT_TYPE_BIN;
use std::sync::Arc;
use tokio::task::JoinHandle;

pub struct Shared {}

pub async fn start_query_forwarder() {
    let binds = &CONFIG.broker().inbound_http_server_bind_address;
    let binds = binds
        .iter()
        .map(|addr| tokio::spawn(start_server(addr.to_string())));
    let binds: Vec<JoinHandle<()>> = binds.collect();
    let result = try_join_all(binds).await;
    if result.is_err() {
        panic!("start_query_forwarder: {}", result.err().unwrap());
    }
}

async fn start_server(addr: String) {
    info!("Starting query forwarder on {}.", &addr);

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

    let router = router.route("/query/:pr_public_key", post(forwarder));
    let router = router.route("/query/0x:pr_public_key", post(forwarder));

    router
}

async fn forwarder(Path(pr_public_key): Path<String>, body: Bytes) -> impl IntoResponse {
    debug!("Incoming query for {}", pr_public_key);
    let lw_map = LW_MAP.clone();
    let lw_map = lw_map.read().await;
    let lw = lw_map.get(&pr_public_key);
    if lw.is_none() {
        return (StatusCode::NOT_FOUND, Bytes::new());
    }
    let lw = lw.unwrap();
    let lw = lw.clone();
    drop(lw_map);

    match lw.status {
        KnownLocalWorkerStatus::Active => {
            let url = format!("http://{}:{}/", &lw.hostname, &lw.forwarder_port);
            debug!(
                "Incoming query matched for local worker ({}/{}) as {}",
                &lw.instance_name, &pr_public_key, &url
            );
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
        KnownLocalWorkerStatus::Lost => (StatusCode::BAD_GATEWAY, Bytes::new()),
        KnownLocalWorkerStatus::Dead => (StatusCode::NOT_FOUND, Bytes::new()),
    }
}
