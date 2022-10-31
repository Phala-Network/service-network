use crate::{WorkerRuntimeStatus, CONFIG, REQ_CLIENT, WR};
use axum::extract::Path;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Extension, Router};
use bytes::Bytes;
use futures::future::try_join_all;
use hyper::Server;
use log::{debug, info, warn};
use phactory_api::prpc::server::ProtoError;
use phactory_api::prpc::Message;
use prost::DecodeError;
use reqwest::header::CONTENT_TYPE;
use reqwest::StatusCode;
use service_network::utils::CONTENT_TYPE_BIN;
use std::iter::Iterator;
use std::string::{String, ToString};
use std::sync::Arc;
use tokio::task::JoinHandle;

pub static ALLOWED_METHODS: &[&'static str] = &["GetInfo", "ContractQuery"];

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

    let router = router.route("/:method", post(forwarder));

    router
}

async fn forwarder(Path(method): Path<String>, body: Bytes) -> impl IntoResponse {
    let wr = WR.read().await;
    let status = wr.status.clone();
    drop(wr);

    match status {
        WorkerRuntimeStatus::Started => {
            if !(ALLOWED_METHODS.contains(&method.as_str())) {
                debug!("Filtered disallowed method {}", method);
                return (StatusCode::NOT_FOUND, Bytes::new());
            }

            let wr = WR.read().await;
            let url = format!("{}/prpc/PhactoryAPI.{}", &wr.pruntime_rpc_prefix, method);
            drop(wr);
            let req = REQ_CLIENT
                .post(url.clone())
                .header(CONTENT_TYPE, CONTENT_TYPE_BIN)
                .body(body)
                .send()
                .await;
            match req {
                Ok(res) => {
                    let status = res.status() as StatusCode;
                    if status.eq(&StatusCode::OK) {
                        (status, res.bytes().await.unwrap_or_default())
                    } else if status.eq(&StatusCode::BAD_REQUEST) {
                        let buffer = res.bytes().await.unwrap_or_default();
                        let decoded: Result<ProtoError, DecodeError> =
                            Message::decode(buffer.clone());
                        if decoded.is_err() {
                            debug!(
                                "Filtered invalid server error from {}: {}",
                                url,
                                String::from_utf8_lossy(&buffer)
                            );
                            (status, Bytes::new())
                        } else {
                            (status, buffer)
                        }
                    } else {
                        debug!(
                            "Error {} from {}: {}",
                            status,
                            url,
                            res.text().await.unwrap_or_default()
                        );
                        (status, Bytes::new())
                    }
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
