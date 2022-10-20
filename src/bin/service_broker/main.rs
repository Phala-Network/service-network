pub mod local_worker;
pub mod mgmt;
pub mod outbound;
pub mod query_forwarder;

use crate::local_worker::{
    local_worker_manager, LocalWorkerManagerChannelMessage, LocalWorkerManagerChannelMessageSender,
    WrappedLocalWorkerMap,
};
use crate::LocalWorkerManagerChannelMessage::ShouldCheckPeerHealth;
use env_logger::{Builder as LoggerBuilder, Target};
use log::{debug, info};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use service_network::config::{PeerConfig, PeerRole, BROKER_HEALTH_CHECK_INTERVAL};
use service_network::peer::{my_ipv4_interfaces, SERVICE_PSN_BROKER};
use service_network::runtime::AsyncRuntimeContext;
use service_network::utils::join_handles;
use std::collections::{BTreeMap, HashMap};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::channel;
use tokio::sync::RwLock;
use tokio::time::sleep;

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
    pub static ref RT_CTX: AsyncRuntimeContext = AsyncRuntimeContext::new(CONFIG.clone());
    pub static ref LW_MAP: WrappedLocalWorkerMap = Arc::new(RwLock::new(BTreeMap::new()));
}

#[tokio::main]
async fn main() {
    let mut builder = LoggerBuilder::from_default_env();
    builder.target(Target::Stdout);
    builder.init();

    info!(
        "pSN service broker version {}-{}",
        VERSION.as_str(),
        GIT_REVISION.as_str()
    );
    debug!("Staring service broker with config: {:?}", &*CONFIG);

    let (tx, rx) = channel::<LocalWorkerManagerChannelMessage>(1024);

    join_handles(vec![
        tokio::spawn(broker()),
        tokio::spawn(outbound::start(&RT_CTX)),
        tokio::spawn(mgmt::start_server(tx.clone(), &RT_CTX, &CONFIG)),
        tokio::spawn(local_worker_manager(tx.clone(), rx)),
        tokio::spawn(check_peer_health_loop(tx.clone())),
    ])
    .await;
}

async fn broker() {
    // let pm = &RT_CTX.peer_manager;
    let mdns = ServiceDaemon::new().expect("Could not create service daemon");
    register_service(&mdns, &RT_CTX).await;
    // pm.browse_local_workers(&mdns, &RT_CTX).await;
}

async fn register_service(mdns: &ServiceDaemon, _ctx: &AsyncRuntimeContext) {
    let common_config = &CONFIG.common;
    let broker_config = CONFIG.broker();

    let my_addrs: Vec<Ipv4Addr> = my_ipv4_interfaces().iter().map(|i| i.ip).collect();

    // in => instance name
    // i => instance_id
    // mp => management port
    // c => cost
    // oa => outbound_bind_addresses
    // o => inbound_http_server_accessible_address_prefix
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

async fn check_peer_health_loop(tx: LocalWorkerManagerChannelMessageSender) {
    loop {
        sleep(Duration::from_millis(BROKER_HEALTH_CHECK_INTERVAL)).await;
        let _ = tx.clone().send(ShouldCheckPeerHealth).await;
    }
}
