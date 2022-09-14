pub mod inbound;
pub mod outbound;

use env_logger::{Builder as LoggerBuilder, Target};
use log::{debug, info};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use psn_peer::config::{PeerConfig, PeerRole};
use psn_peer::peer::{my_ipv4_interfaces, PeerManager, SERVICE_PSN_BROKER};
use psn_peer::runtime::{AsyncRuntimeContext, WrappedAsyncRuntimeContext};
use std::collections::HashMap;
use std::net::Ipv4Addr;

#[tokio::main]
async fn main() {
    let mut builder = LoggerBuilder::from_default_env();
    builder.target(Target::Stdout);
    builder.init();

    let version = env!("CARGO_PKG_VERSION").to_string();
    let git_revision = option_env!("PROJECT_GIT_REVISION")
        .unwrap_or("dev")
        .to_string();

    info!("pSN service broker version {}-{}", version, git_revision);

    let config = PeerConfig::build_from_env(PeerRole::PrBroker(None), version, git_revision);
    debug!("Staring service broker with config: {config:?}");

    let ctx = AsyncRuntimeContext::new_wrapped(config);

    let _ = tokio::join!(
        tokio::spawn(broker(ctx.clone())),
        tokio::spawn(PeerManager::browse_brokers(ctx.clone()))
    );
}

async fn broker(ctx: WrappedAsyncRuntimeContext) {
    register_service(ctx.clone()).await;
    // todo
}

async fn register_service(ctx_w: WrappedAsyncRuntimeContext) {
    let ctx = ctx_w.clone();
    let ctx = ctx.read().await;
    let config = &ctx.config;
    let common_config = &config.common;
    let broker_config = config.broker();

    let mdns = ServiceDaemon::new().expect("Could not create service daemon");
    let my_addrs: Vec<Ipv4Addr> = my_ipv4_interfaces().iter().map(|i| i.ip).collect();
    let service_info = ServiceInfo::new(
        SERVICE_PSN_BROKER,
        common_config.instance_name.as_str(),
        config.host_name().as_str(),
        &my_addrs[..],
        common_config.mgmt_port,
        Some(HashMap::from([
            ("in".to_string(), format!("{}", common_config.instance_name)),
            ("i".to_string(), format!("{}", config.instance_id)),
            ("c".to_string(), format!("{}", broker_config.cost)),
            ("mp".to_string(), format!("{}", common_config.mgmt_port)),
            (
                "op".to_string(),
                format!("{}", broker_config.outbound_socks_port),
            ),
            (
                "o".to_string(),
                format!(
                    "{}",
                    broker_config.inbound_http_server_accessible_address_prefix
                ),
            ),
        ])),
    )
    .expect("Failed to build service info");
    mdns.register(service_info.clone())
        .expect("Failed to register mDNS service");

    info!(
        "[register_service] Registered service for {}: {:?}",
        config.mdns_fullname(),
        &service_info
    );
}
