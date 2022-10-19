pub mod local_worker;

use crate::LocalWorkerManagerChannelMessageSender;
use axum::routing::put;
use axum::{Extension, Router};
use hyper::Server;
use log::info;
use service_network::config::PeerConfig;
use service_network::mgmt_types::{MyIdentity, R_V0_LOCAL_WORKER_KEEPALIVE};
use service_network::runtime::AsyncRuntimeContext;
use std::sync::Arc;

pub struct BrokerMgmtShared {
    pub tx: LocalWorkerManagerChannelMessageSender,
    pub config: PeerConfig,
    pub my_id: String,
}

pub async fn start_server(
    tx: LocalWorkerManagerChannelMessageSender,
    _ctx: &AsyncRuntimeContext,
    config: &'static PeerConfig,
) {
    let my_id = MyIdentity {
        instance_name: config.common.instance_name.to_string(),
        instance_id: config.instance_id.to_string(),
    };
    let my_id = serde_json::to_string(&my_id).unwrap();
    let shared = BrokerMgmtShared {
        tx,
        config: config.clone(),
        my_id,
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
        R_V0_LOCAL_WORKER_KEEPALIVE,
        put(local_worker::handle_keepalive),
    );

    router
}
