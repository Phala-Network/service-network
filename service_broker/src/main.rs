pub mod inbound;
pub mod outbound;

use env_logger::{Builder as LoggerBuilder, Target};
use log::{debug, info};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use psn_peer::config::{PeerConfig, PeerRole};
use psn_peer::peer::{my_ipv4_interfaces, PeerManager, SERVICE_PSN_BROKER};
use psn_peer::runtime::{AsyncRuntimeContext, WrappedAsyncRuntimeContext, WrappedRuntime};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use tokio::runtime::Builder;
use tokio::sync::RwLock;

fn main() {
    let mut builder = LoggerBuilder::from_default_env();
    builder.target(Target::Stdout);
    builder.init();

    let version = env!("CARGO_PKG_VERSION").to_string();
    let git_revision = option_env!("PROJECT_GIT_REVISION")
        .unwrap_or("dev")
        .to_string();

    info!("pSN service broker version {}-{}", version, git_revision);

    let config = PeerConfig::build_from_env(PeerRole::PrBroker, version, git_revision);
    debug!("Staring service broker with config: {config:?}");

    let rt_w = Arc::new(RwLock::new(
        Builder::new_multi_thread().enable_all().build().unwrap(),
    ));

    let rt = rt_w.try_read().unwrap();
    let _guard = rt.enter();
    drop(rt);

    let ctx_wrapper = AsyncRuntimeContext::init(config);

    AsyncRuntimeContext::block_on_all(
        rt_w.clone(),
        vec![
            AsyncRuntimeContext::spawn(ctx_wrapper.clone(), rt_w.clone(), init_broker),
            AsyncRuntimeContext::spawn(
                ctx_wrapper.clone(),
                rt_w.clone(),
                PeerManager::browse_brokers,
            ),
        ],
    );
}

async fn init_broker(ctx_w: WrappedAsyncRuntimeContext, rt_w: WrappedRuntime) {
    register_service(ctx_w.clone(), rt_w.clone()).await;
}

async fn register_service(ctx_w: WrappedAsyncRuntimeContext, _rt_w: WrappedRuntime) {
    let ctx = ctx_w.clone();
    let ctx = ctx.read().await;
    let config = &ctx.config.clone();
    let config = config.read().await;
    let common_config = &config.common;
    let broker_config = &config.broker;
    let broker_config = &broker_config.as_deref().unwrap();

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
