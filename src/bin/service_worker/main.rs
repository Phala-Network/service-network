pub mod public_forwarder;
pub mod runtime;

use crate::public_forwarder::start_public_forwarder;
use crate::runtime::*;
use crate::WorkerRuntimeChannelMessage::*;
use env_logger::{Builder as LoggerBuilder, Target};
use log::{debug, info};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use phactory_api::pruntime_client::{new_pruntime_client, PRuntimeClient};
use service_network::config::{PeerConfig, PeerRole};
use service_network::peer::local_worker::BrokerPeerUpdateSender;
use service_network::peer::{my_ipv4_interfaces, SERVICE_PSN_LOCAL_WORKER};
use service_network::runtime::AsyncRuntimeContext;
use service_network::utils::join_handles;
use std::collections::HashMap;
use std::net::Ipv4Addr;

#[macro_use]
extern crate lazy_static;

lazy_static! {
    pub static ref VERSION: String = env!("CARGO_PKG_VERSION").to_string();
    pub static ref GIT_REVISION: String = option_env!("PROJECT_GIT_REVISION")
        .unwrap_or("dev")
        .to_string();
    pub static ref CONFIG: PeerConfig = PeerConfig::build_from_env(
        PeerRole::PrLocalWorker(None),
        VERSION.to_string(),
        GIT_REVISION.to_string(),
    );
    pub static ref RT_CTX: AsyncRuntimeContext = AsyncRuntimeContext::new(CONFIG.clone());
    pub static ref PRUNTIME_CLIENT: PRuntimeClient =
        new_pruntime_client(CONFIG.local_worker().pruntime_address.to_string());
    pub static ref MDNS: ServiceDaemon = ServiceDaemon::new().expect("Failed to create daemon");
    pub static ref WR: WrappedWorkerRuntime =
        WorkerRuntime::new_wrapped(&MDNS, &RT_CTX, &PRUNTIME_CLIENT);
    pub static ref REQ_CLIENT: reqwest::Client = reqwest::Client::new();
}

#[tokio::main]
async fn main() {
    let mut builder = LoggerBuilder::from_default_env();
    builder.target(Target::Stdout);
    builder.init();

    info!(
        "pSN local worker version {}-{}",
        VERSION.as_str(),
        GIT_REVISION.as_str()
    );
    debug!("Staring local worker broker with config: {:?}", &*CONFIG);

    let (update_best_peer_tx, _) = tokio::sync::mpsc::channel(1024);
    let (rt_tx, rt_rx) = tokio::sync::mpsc::channel(1024);

    join_handles(vec![
        tokio::spawn(handle_runtime_events(
            update_best_peer_tx.clone(),
            rt_tx.clone(),
            rt_rx,
        )),
        tokio::spawn(start_public_forwarder()),
        tokio::spawn(check_pruntime_health(rt_tx.clone())),
        tokio::spawn(check_current_broker_health_loop(rt_tx.clone())),
    ])
    .await;
}

async fn handle_runtime_events(
    tx: BrokerPeerUpdateSender,
    rt_tx: WorkerRuntimeChannelMessageSender,
    mut rx: WorkerRuntimeChannelMessageReceiver,
) {
    while let Some(msg) = rx.recv().await {
        match msg {
            ShouldSetPRuntimeFailed(msg) => {
                let wr = WR.read().await;
                wr.handle_pruntime_failure(rt_tx.clone(), msg).await;
                drop(wr);
            }
            ShouldSetBrokerFailed(msg) => {
                let wr = WR.read().await;
                wr.handle_broker_failure(rt_tx.clone(), msg).await;
                drop(wr);
            }
            ShouldUpdateInfo(info) => {
                let mut wr = WR.write().await;
                wr.handle_update_info(rt_tx.clone(), info).await;
                drop(wr);
            }
            ShouldUpdateStatus(s) => {
                let mut wr = WR.write().await;
                let pr = wr.prc;
                wr.handle_update_status(s, tx.clone(), rt_tx.clone(), pr)
                    .await;
                drop(wr);
            }
            ShouldLockBroker(peer) => {
                let mut wr = WR.write().await;
                wr.handle_lock_peer(peer.clone(), rt_tx.clone()).await;
                drop(wr);
            }
        }
    }
}

async fn register_service(mdns: &ServiceDaemon, _ctx: &AsyncRuntimeContext) {
    let common_config = &CONFIG.common;
    let _worker_config = CONFIG.local_worker();

    let my_addrs: Vec<Ipv4Addr> = my_ipv4_interfaces().iter().map(|i| i.ip).collect();

    // in => instance name
    // i => instance_id
    // mp => management port
    let service_info = ServiceInfo::new(
        SERVICE_PSN_LOCAL_WORKER,
        common_config.instance_name.as_str(),
        CONFIG.host_name().as_str(),
        &my_addrs[..],
        common_config.mgmt_port,
        Some(HashMap::from([
            ("in".to_string(), format!("{}", common_config.instance_name)),
            ("i".to_string(), format!("{}", CONFIG.instance_id)),
            ("mp".to_string(), format!("{}", common_config.mgmt_port)),
            // todo: worker info
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
