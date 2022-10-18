pub mod local_worker;

use crate::LocalWorkerManagerChannelMessageSender;
use axum::routing::put;
use axum::{Extension, Router};
use hyper::Server;
use log::info;
use serde::{Deserialize, Serialize};
use service_network::config::PeerConfig;
use service_network::runtime::AsyncRuntimeContext;
use std::sync::Arc;

pub struct BrokerMgmtShared {
    pub tx: LocalWorkerManagerChannelMessageSender,
    pub config: PeerConfig,
}

pub async fn start_server(
    tx: LocalWorkerManagerChannelMessageSender,
    _ctx: &AsyncRuntimeContext,
    config: &'static PeerConfig,
) {
    let shared = BrokerMgmtShared {
        tx,
        config: config.clone(),
    };
    tokio::spawn(async move {
        let router = create_router();
        // TODO: add RSA identity for peer management api
        let bind_addr = config.common.mgmt_port;
        let bind_addr = format!("0.0.0.0:{}", bind_addr);
        info!("Starting management API on {}...", &bind_addr);
        let bind_addr = &bind_addr.parse().unwrap();
        let router = router.layer(Extension(Arc::new(shared)));
        Server::bind(bind_addr)
            .serve(router.into_make_service())
            .await
            .unwrap();
    })
    .await
    .expect("Failed to start management server");
}

fn create_router() -> Router {
    let router = Router::new();

    // Internal API v0
    let router = router.route(
        "/v0/local_worker/keepalive",
        put(local_worker::handle_keepalive),
    );

    router
}

#[derive(Serialize, Deserialize)]
pub struct MyIdentity {
    instance_name: String,
    instance_id: String,
}
