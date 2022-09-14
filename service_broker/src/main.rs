pub mod inbound;
pub mod outbound;

use env_logger::{Builder as LoggerBuilder, Target};
use log::{debug, info, trace};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use psn_peer::config::{BrokerConfig, PeerConfig, PeerRole};
use psn_peer::peer::{my_ipv4_interfaces, PeerManager, SERVICE_PSN_BROKER};
use psn_peer::runtime::{AsyncRuntimeContext, WrappedAsyncRuntimeContext};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr;
use tokio::runtime::Handle;
use trust_dns_resolver::config::{ResolverConfig, ResolverOpts};
use trust_dns_resolver::name_server::{GenericConnection, GenericConnectionProvider};
use trust_dns_resolver::{AsyncResolver, Resolver};

#[macro_use]
extern crate lazy_static;

lazy_static! {
    pub static ref VERSION: String = env!("CARGO_PKG_VERSION").to_string();
    pub static ref GIT_REVISION: String = option_env!("PROJECT_GIT_REVISION")
        .unwrap_or("dev")
        .to_string();
    pub static ref CONFIG: PeerConfig = PeerConfig::build_from_env(
        PeerRole::PrBroker(None),
        VERSION.to_string(),
        GIT_REVISION.to_string(),
    );
}

#[tokio::main]
async fn main() {
    let mut builder = LoggerBuilder::from_default_env();
    builder.target(Target::Stdout);
    builder.init();

    info!(
        "pSN service broker version {:?}-{:?}",
        VERSION.as_str(),
        GIT_REVISION.as_str()
    );
    debug!("Staring service broker with config: {:?}", &*CONFIG);

    let ctx = AsyncRuntimeContext::new_wrapped(CONFIG.clone());

    let _ = tokio::join!(
        broker(ctx.clone()),
        tokio::spawn(PeerManager::browse_brokers(ctx.clone()))
    );
}

async fn broker(ctx: WrappedAsyncRuntimeContext) {
    register_service(ctx.clone()).await;
    outbound::start(ctx.clone()).await;
    trace!("11111");
    // todo
}

async fn register_service(_ctx: WrappedAsyncRuntimeContext) {
    let common_config = &CONFIG.common;
    let broker_config = CONFIG.broker();

    let mdns = ServiceDaemon::new().expect("Could not create service daemon");
    let my_addrs: Vec<Ipv4Addr> = my_ipv4_interfaces().iter().map(|i| i.ip).collect();
    let service_info = ServiceInfo::new(
        SERVICE_PSN_BROKER,
        common_config.instance_name.as_str(),
        CONFIG.host_name().as_str(),
        &my_addrs[..],
        common_config.mgmt_port,
        Some(HashMap::from([
            ("in".to_string(), format!("{}", common_config.instance_name)),
            ("i".to_string(), format!("{}", CONFIG.instance_id)),
            ("c".to_string(), format!("{}", broker_config.cost)),
            ("mp".to_string(), format!("{}", common_config.mgmt_port)),
            (
                "oa".to_string(),
                format!("{}", broker_config.outbound_bind_addresses.join(",")),
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
        CONFIG.mdns_fullname(),
        &service_info
    );
}
