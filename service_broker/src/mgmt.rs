use log::info;
use psn_peer::config::PeerConfig;
use psn_peer::mgmt::{create_router, start_with_router};
use psn_peer::runtime::AsyncRuntimeContext;

pub async fn start_server(_ctx: &AsyncRuntimeContext, config: &'static PeerConfig) {
    tokio::spawn(async move {
        let router = create_router();
        // TODO: management port can be called by discovered peers only
        let bind_addr = config.common.mgmt_port;
        let bind_addr = format!("0.0.0.0:{}", bind_addr);
        info!("Starting management API on {}...", &bind_addr);
        let bind_addr = &bind_addr.parse().unwrap();
        start_with_router(bind_addr, router, config).await;
    })
    .await
    .expect("Failed to start management server");
}
