use crate::WorkerRuntimeStatus::*;
use crate::{ShouldLockBroker, ShouldSetBrokerFailed, ShouldUpdateStatus, RT_CTX, WR};
use anyhow::{anyhow, Context, Result};
use log::{debug, error, info, warn};
use mdns_sd::ServiceDaemon;
use phactory_api::prpc::{NetworkConfig, PhactoryInfo};
use phactory_api::pruntime_client::PRuntimeClient;
use service_network::peer::local_worker::{BrokerPeerUpdateSender, WrappedBrokerPeer};
use service_network::runtime::AsyncRuntimeContext;
use std::fmt::Debug;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone)]
pub enum WorkerRuntimeChannelMessage {
    ShouldUpdateInfo(PhactoryInfo),
    ShouldSetPRuntimeFailed(String),
    ShouldSetBrokerFailed(String),
    ShouldUpdateStatus(WorkerRuntimeStatus),
    ShouldLockBroker(WrappedBrokerPeer),
}

pub type WorkerRuntimeChannelMessageSender = Sender<WorkerRuntimeChannelMessage>;
pub type WorkerRuntimeChannelMessageReceiver = Receiver<WorkerRuntimeChannelMessage>;

#[derive(Debug, Clone)]
pub enum WorkerRuntimeStatus {
    Starting,
    WaitingForBroker,
    PendingProvision,
    Started,
    Failed(WorkerRuntimeFailReason),
}

#[derive(Debug, Clone)]
pub struct WorkerRuntimeFailReason {
    pub broker: Option<String>,
    pub pr: Option<String>,
}

pub type WrappedWorkerRuntime = Arc<RwLock<WorkerRuntime>>;

pub struct WorkerRuntime {
    pub rt_ctx: &'static AsyncRuntimeContext,
    pub prc: &'static PRuntimeClient,
    pub status: WorkerRuntimeStatus,
    pub initial_info: Option<PhactoryInfo>,
    pub last_info: Option<PhactoryInfo>,
    pub pr_rpc_port: u32,
    pub peer_browser_started: bool,
    pub best_broker: Option<WrappedBrokerPeer>,
    pub best_broker_name: Option<String>,
}

impl WorkerRuntime {
    pub fn new(rt_ctx: &'static AsyncRuntimeContext, prc: &'static PRuntimeClient) -> Self {
        Self {
            rt_ctx,
            prc,
            status: Starting,
            initial_info: None,
            last_info: None,
            pr_rpc_port: 0,
            peer_browser_started: false,
            best_broker: None,
            best_broker_name: None,
        }
    }

    pub fn new_wrapped(
        rt_ctx: &'static AsyncRuntimeContext,
        prc: &'static PRuntimeClient,
    ) -> WrappedWorkerRuntime {
        Arc::new(RwLock::new(Self::new(rt_ctx, prc)))
    }

    pub async fn handle_update_info(
        &mut self,
        rt_tx: WorkerRuntimeChannelMessageSender,
        info: PhactoryInfo,
    ) {
        let ir = (&info).clone();
        let current_status = (&self.status).clone();
        let pr_rpc_port = &ir.network_status.unwrap_or_default().public_rpc_port;

        if pr_rpc_port.is_none() {
            self.pr_rpc_port = 0;
            let rt_tx = rt_tx.clone();
            let _ = rt_tx
                .clone()
                .send(WorkerRuntimeChannelMessage::ShouldSetPRuntimeFailed(
                    "Public port not enabled in pRuntime!".to_string(),
                ))
                .await;
            return;
        }

        self.pr_rpc_port = pr_rpc_port.unwrap();
        self.last_info = Some(info.clone());

        match current_status {
            Starting => {
                self.initial_info = Some(info);
                let rt_tx = rt_tx.clone();
                let _ = rt_tx
                    .clone()
                    .send(ShouldUpdateStatus(WaitingForBroker))
                    .await;
            }
            Failed(_) => {
                info!("Trying to recover from failure in 6s...");
                sleep(Duration::from_secs(6)).await;
                self.recover_from_failure(rt_tx.clone()).await;
            }
            _ => {}
        }
    }

    pub async fn recover_from_failure(&self, rt_tx: WorkerRuntimeChannelMessageSender) {
        let current_status = (&self.status).clone();
        match current_status {
            Failed(_) => {
                debug!("Resetting runtime status to Starting...");
                let _ = rt_tx.clone().send(ShouldUpdateStatus(Starting)).await;
            }
            _ => {
                debug!("Triggering `recover_from_failure` from normal status, ignoring.");
            }
        }
    }

    pub async fn handle_lock_peer(
        &mut self,
        peer: WrappedBrokerPeer,
        rt_tx: WorkerRuntimeChannelMessageSender,
    ) {
        let np = peer.clone();
        let np = np.lock().await;
        let name = np.instance_name.clone();
        drop(np);

        self.best_broker = Some(peer.clone());
        self.best_broker_name = Some(name.clone());
        let _ = rt_tx
            .clone()
            .send(ShouldUpdateStatus(PendingProvision))
            .await;
        info!("Locked self to broker {}.", name);
    }

    pub async fn handle_update_status(
        &mut self,
        new_status: WorkerRuntimeStatus,
        tx: BrokerPeerUpdateSender,
        rt_tx: WorkerRuntimeChannelMessageSender,
        pr: &PRuntimeClient,
    ) {
        let old_status = (&self.status).clone();
        info!(
            "Changing worker status from {:?} to {:?}...",
            &old_status, &new_status
        );
        match new_status {
            Starting => {
                info!("Staring worker runtime...");
            }
            WaitingForBroker => {
                if !(self.peer_browser_started) {
                    tokio::spawn(start_peer_browser(tx.clone(), &RT_CTX));
                    self.peer_browser_started = true;
                }
                tokio::spawn(wait_for_broker_peer(rt_tx.clone()));
            }
            PendingProvision => {
                // todo: transition state left for sideVM
                if let Err(e) = set_pruntime_network_with_peer(
                    (&self.best_broker).as_ref().unwrap().clone(),
                    pr,
                )
                .await
                {
                    let _ = rt_tx
                        .clone()
                        .send(ShouldSetBrokerFailed(format!("{}", e)))
                        .await;
                } else {
                    let _ = rt_tx.clone().send(ShouldUpdateStatus(Started)).await;
                };
            }
            Started => {}
            Failed(_) => {}
        }
        self.status = new_status;
    }

    pub async fn handle_broker_failure(
        &self,
        rt_tx: WorkerRuntimeChannelMessageSender,
        msg: String,
    ) {
        let current_status = (&self.status).clone();
        match current_status {
            Failed(reason) => {
                if (&reason.broker).is_some() {
                    let current_msg = &reason.broker.clone().unwrap();
                    if current_msg.eq(&msg) {
                        debug!("Ignored coming fail reason from broker: {}", &msg);
                        return;
                    }
                }
                let mut reason = reason.clone();
                reason.broker = Some(msg);
                let _ = rt_tx.clone().send(ShouldUpdateStatus(Failed(reason))).await;
            }
            _ => {
                let _ = rt_tx
                    .clone()
                    .send(ShouldUpdateStatus(Failed(WorkerRuntimeFailReason {
                        pr: None,
                        broker: Some(msg),
                    })))
                    .await;
            }
        };
    }

    pub async fn handle_pruntime_failure(
        &self,
        rt_tx: WorkerRuntimeChannelMessageSender,
        msg: String,
    ) {
        let current_status = (&self.status).clone();
        match current_status {
            Failed(reason) => {
                if (&reason.pr).is_some() {
                    let current_msg = &reason.pr.clone().unwrap();
                    if current_msg.eq(&msg) {
                        debug!("Ignored coming fail reason from pRuntime: {}", &msg);
                        return;
                    }
                }
                let mut reason = reason.clone();
                reason.pr = Some(msg);
                let _ = rt_tx.clone().send(ShouldUpdateStatus(Failed(reason))).await;
            }
            _ => {
                let _ = rt_tx
                    .clone()
                    .send(WorkerRuntimeChannelMessage::ShouldUpdateStatus(
                        WorkerRuntimeStatus::Failed(WorkerRuntimeFailReason {
                            broker: None,
                            pr: Some(msg),
                        }),
                    ))
                    .await;
            }
        };
    }
}

pub async fn wait_for_broker_peer(rt_tx: WorkerRuntimeChannelMessageSender) {
    info!("Starting waiting for broker peer...");
    let mut loop_num: u8 = 0;
    loop {
        let wr = WR.read().await;
        match wr.status {
            WorkerRuntimeStatus::WaitingForBroker => {
                let pm = &RT_CTX.peer_manager.broker;
                let mut pm = pm.lock().await;
                let peer = pm.verify_best_instance().await;
                drop(pm);
                if peer.is_err() {
                    let e = peer.err().unwrap();
                    panic!("Failed to verify selected candidate: {}", e);
                } else {
                    let peer = peer.unwrap();
                    if peer.is_some() {
                        let peer = peer.unwrap();
                        loop_num += 1;
                        if loop_num >= 15 {
                            let _ = rt_tx.clone().send(ShouldLockBroker(peer)).await;
                            return;
                        }
                    } else {
                        debug!("Broker not found, waiting...");
                        loop_num += 1;
                    }
                }
            }
            _ => {
                return;
            }
        }
        sleep(Duration::from_secs(1)).await;
        drop(wr);
    }
}

pub async fn check_pruntime_health(rt_tx: WorkerRuntimeChannelMessageSender) {
    info!("Starting pinging pRuntime for health check...");
    loop {
        let wr = WR.read().await;
        let duration = Duration::from_secs(match wr.status {
            WorkerRuntimeStatus::Starting => 1,
            WorkerRuntimeStatus::WaitingForBroker => 15,
            WorkerRuntimeStatus::PendingProvision => 15,
            WorkerRuntimeStatus::Started => 6,
            WorkerRuntimeStatus::Failed(_) => 3,
        });
        drop(wr);

        sleep(duration).await;
        let wr = WR.read().await;
        let pr = wr.prc;
        let info = pr.get_info(()).await;
        drop(wr);
        match info {
            Ok(info) => {
                let _ = rt_tx
                    .clone()
                    .send(WorkerRuntimeChannelMessage::ShouldUpdateInfo(info))
                    .await;
            }
            Err(err) => {
                let err = format!("{:?}", err);
                debug!("Error while fetching info from pRuntime: {:?}", &err);
                let _ = rt_tx
                    .clone()
                    .send(WorkerRuntimeChannelMessage::ShouldSetPRuntimeFailed(err))
                    .await;
            }
        }
    }
}

pub async fn start_peer_browser(tx: BrokerPeerUpdateSender, ctx: &AsyncRuntimeContext) {
    let mdns = ServiceDaemon::new().expect("Failed to create daemon");
    let pm = &RT_CTX.peer_manager;
    crate::register_service(&mdns, &ctx).await;
    pm.browse_brokers(&mdns, tx.clone(), &RT_CTX).await;
}

async fn set_pruntime_network_with_peer(
    peer: WrappedBrokerPeer,
    pr: &PRuntimeClient,
) -> Result<()> {
    let p = peer.lock().await;
    let si = &p.service_info;
    let props = si.get_properties();

    let oa = props.get("oa").context("Invalid outbound address").unwrap();
    let oa = oa.split(",").map(|str| str.trim()).collect::<Vec<&str>>();
    let oa = oa.get(0).context("Invalid outbound address").unwrap();
    let oa = oa.split(":").collect::<Vec<&str>>();
    let oap = *oa.get(1).context("Invalid outbound port").unwrap();
    let oa = *oa.get(0).context("Invalid outbound port").unwrap();
    let oa = if oa.eq("0.0.0.0") {
        let addr = si
            .get_addresses()
            .clone()
            .into_iter()
            .next()
            .context("Invalid outbound address")
            .unwrap();
        addr.octets().map(|i| i.to_string()).join(".")
    } else {
        oa.to_string()
    };
    let all_proxy = format!("socks5://{}:{}", oa, oap);
    drop(p);

    info!("Setting pRuntime outbound proxy to {}.", &all_proxy);
    if let Err(e) = pr
        .config_network(NetworkConfig {
            all_proxy,
            i2p_proxy: "".to_string(),
        })
        .await
    {
        return Err(anyhow!("Failed to configure pRuntime network{:?}", e));
    }
    Ok(())
}
