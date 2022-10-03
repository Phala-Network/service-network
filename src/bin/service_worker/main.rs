use anyhow::Result;
use env_logger::{Builder as LoggerBuilder, Target};
use log::{debug, info, warn};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use phactory_api::pruntime_client::{new_pruntime_client, PRuntimeClient};
use service_network::config::{PeerConfig, PeerRole};
use service_network::peer::{
    my_ipv4_interfaces, BrokerPeerUpdate, BrokerPeerUpdateReceiver, BrokerPeerUpdateSender,
    SERVICE_PSN_LOCAL_WORKER,
};
use service_network::runtime::AsyncRuntimeContext;
use service_network::worker::runtime::WorkerRuntimeChannelMessage::{
    ShouldUpdateInfo, ShouldUpdateStatus,
};
use service_network::worker::runtime::{
    WorkerRuntime, WorkerRuntimeChannelMessage, WorkerRuntimeStatus, WrappedWorkerRuntime,
};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use tokio::sync::mpsc::{Receiver, Sender};

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
    pub static ref WR: WrappedWorkerRuntime = WorkerRuntime::new_wrapped(&RT_CTX, &PRUNTIME_CLIENT);
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

    let (update_best_peer_tx, update_best_peer_rx) = tokio::sync::mpsc::channel(32);
    let (rt_tx, rt_rx) = tokio::sync::mpsc::channel(1024);

    let handle1 = tokio::spawn(handle_update_best_peer(update_best_peer_rx, CONFIG.clone()));
    let handle2 = tokio::spawn(runtime(update_best_peer_tx.clone(), rt_tx.clone(), rt_rx));

    let wr = WR.read().await;
    let pr = wr.prc;
    let info = pr
        .get_info(())
        .await
        .expect("Failed to get info from pruntime");
    drop(wr);
    let _ = rt_tx
        .clone()
        .send(WorkerRuntimeChannelMessage::ShouldUpdateInfo(info))
        .await;

    let _ = tokio::join!(handle1, handle2);
}

async fn local_worker(
    tx: BrokerPeerUpdateSender,
    rt_tx: Sender<WorkerRuntimeChannelMessage>,
    ctx: &AsyncRuntimeContext,
) {
    let mdns = ServiceDaemon::new().expect("Failed to create daemon");
    let pm = &RT_CTX.peer_manager;
    register_service(&mdns, &ctx).await;
    pm.browse_brokers(&mdns, tx.clone(), rt_tx.clone(), &RT_CTX)
        .await;
}

async fn runtime(
    tx: BrokerPeerUpdateSender,
    rt_tx: Sender<WorkerRuntimeChannelMessage>,
    mut rx: Receiver<WorkerRuntimeChannelMessage>,
) {
    while let Some(msg) = rx.recv().await {
        match msg {
            ShouldUpdateInfo(i) => {
                let mut wr = WR.write().await;
                wr.handle_update_info(i);
                if matches!(wr.status, WorkerRuntimeStatus::Starting) {
                    let rt_tx = rt_tx.clone();
                    let _ = rt_tx
                        .send(WorkerRuntimeChannelMessage::ShouldUpdateStatus(
                            WorkerRuntimeStatus::WaitingForBroker,
                        ))
                        .await;
                }
                drop(wr);
            }
            ShouldUpdateStatus(s) => {
                let mut wr = WR.write().await;
                wr.handle_update_status(s);
                if matches!(wr.status, WorkerRuntimeStatus::WaitingForBroker) {
                    tokio::spawn(local_worker(tx.clone(), rt_tx.clone(), &RT_CTX));
                }
                drop(wr);
            }
        }
    }
}

async fn set_pruntime_network_with_peer(socks_url: String, _config: PeerConfig) -> Result<()> {
    debug!(
        "Trying to set pRuntime outbound with {}",
        socks_url.as_str()
    );

    Ok(())
}

async fn handle_update_best_peer(mut rx: BrokerPeerUpdateReceiver, config: PeerConfig) {
    while let Some(u) = rx.recv().await {
        match u {
            BrokerPeerUpdate::PeerStatusChanged(name, status) => {
                info!("Broker {} changed its status to {:?}", name, status);
            }
            BrokerPeerUpdate::BestPeerChanged(instance_name, socks_url) => {
                // todo: needs to be throttled
                match tokio::spawn(set_pruntime_network_with_peer(
                    socks_url.to_string(),
                    config.clone(),
                ))
                .await
                {
                    Ok(_) => {
                        info!(
                            "Updated pRuntime networking with ({}):{}",
                            instance_name.as_str(),
                            socks_url.as_str()
                        );
                    }
                    Err(err) => {
                        warn!("Failed to set pRuntime networking: {:?}", err);
                    }
                }
            }
            _ => {}
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
