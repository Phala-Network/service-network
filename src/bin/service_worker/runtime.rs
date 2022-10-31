use crate::WorkerRuntimeStatus::*;
use crate::{
    ShouldLockBroker, ShouldSetBrokerFailed, ShouldUpdateStatus, CONFIG, REQ_CLIENT, RT_CTX, WR,
};
use anyhow::{anyhow, Context, Result};
use log::{debug, error, info};
use mdns_sd::{Error, ServiceDaemon};
use phactory_api::prpc::{NetworkConfig, NetworkConfigResponse, PhactoryInfo};
use phactory_api::pruntime_client::PRuntimeClient;
use reqwest::header::CONTENT_TYPE;
use semver::{Version, VersionReq};
use service_network::config::LOCAL_WORKER_KEEPALIVE_INTERVAL;
use service_network::mgmt_types::{LocalWorkerIdentity, MyIdentity, R_V0_LOCAL_WORKER_KEEPALIVE};
use service_network::peer::local_worker::{BrokerPeerUpdateSender, WrappedBrokerPeer};
use service_network::peer::{my_ipv4_interfaces, SERVICE_PSN_BROKER};
use service_network::runtime::AsyncRuntimeContext;
use service_network::utils::CONTENT_TYPE_JSON;
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use tokio::time::{sleep, Duration};
use urlparse::urlparse;

const PRUNTIME_VERSION_REQ: &str = ">=2.0.0";

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
    pub pruntime_hostname: String,
    pub pruntime_rpc_prefix: String,
    pub peer_browser_started: bool,
    pub public_key: Option<String>,
    pub best_broker: Option<WrappedBrokerPeer>,
    pub best_broker_name: Option<String>,
    pub best_broker_mgmt_url: Option<String>,
    pub keepalive_content: Option<String>,
    pub mdns: &'static ServiceDaemon,
    pub network_status: Option<NetworkConfigResponse>,
}

impl WorkerRuntime {
    pub fn new(
        mdns: &'static ServiceDaemon,
        rt_ctx: &'static AsyncRuntimeContext,
        prc: &'static PRuntimeClient,
    ) -> Self {
        Self {
            rt_ctx,
            prc,
            status: Starting,
            initial_info: None,
            last_info: None,
            pr_rpc_port: 0,
            pruntime_hostname: "".to_string(),
            pruntime_rpc_prefix: "".to_string(),
            peer_browser_started: false,
            public_key: None,
            best_broker: None,
            best_broker_name: None,
            best_broker_mgmt_url: None,
            keepalive_content: None,
            mdns,
            network_status: None,
        }
    }

    pub fn new_wrapped(
        mdns: &'static ServiceDaemon,
        rt_ctx: &'static AsyncRuntimeContext,
        prc: &'static PRuntimeClient,
    ) -> WrappedWorkerRuntime {
        Arc::new(RwLock::new(Self::new(mdns, rt_ctx, prc)))
    }

    pub async fn handle_update_info(
        &mut self,
        rt_tx: WorkerRuntimeChannelMessageSender,
        info: PhactoryInfo,
    ) {
        let ir = (&info).clone();
        let current_status = (&self.status).clone();

        match current_status {
            Starting => {
                self.pruntime_hostname =
                    urlparse(CONFIG.local_worker().pruntime_address.to_string())
                        .hostname
                        .unwrap();

                let version_req = VersionReq::parse(PRUNTIME_VERSION_REQ).unwrap();
                let version = Version::parse(&ir.version).unwrap();
                if !(version_req.matches(&version)) {
                    error!(
                        "pRuntime version unsupported! Requires {}, found {}.",
                        PRUNTIME_VERSION_REQ, &ir.version
                    );
                    let _ = rt_tx
                        .clone()
                        .send(WorkerRuntimeChannelMessage::ShouldSetPRuntimeFailed(
                            "pRuntime version unsupported!".to_string(),
                        ))
                        .await;
                    return;
                }

                let initialized = &ir.initialized;
                let initialized = *initialized;
                if !initialized {
                    let _ = rt_tx
                        .clone()
                        .send(WorkerRuntimeChannelMessage::ShouldSetPRuntimeFailed(
                            "pRuntime not initialized!".to_string(),
                        ))
                        .await;
                    return;
                }

                let network_status = self.prc.get_network_config(()).await;
                if network_status.is_err() {
                    self.network_status = None;
                    let rt_tx = rt_tx.clone();
                    let _ = rt_tx
                        .clone()
                        .send(WorkerRuntimeChannelMessage::ShouldSetPRuntimeFailed(
                            format!("{:?}", network_status.unwrap_err()),
                        ))
                        .await;
                    return;
                }
                let network_status = network_status.unwrap();
                let pr_rpc_port = network_status.public_rpc_port;

                self.network_status = Some(network_status);

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
                self.pruntime_rpc_prefix =
                    format!("http://{}:{}", self.pruntime_hostname, self.pr_rpc_port);

                let public_key = &ir.public_key;
                if public_key.is_none() {
                    let _ = rt_tx
                        .clone()
                        .send(WorkerRuntimeChannelMessage::ShouldSetPRuntimeFailed(
                            "pRuntime has invalid public key!".to_string(),
                        ))
                        .await;
                    return;
                }
                let public_key = public_key.as_ref().unwrap().to_string();
                info!("Initialized for worker 0x{}", &public_key);
                self.public_key = Some(public_key);

                let rt_tx = rt_tx.clone();
                let _ = rt_tx
                    .clone()
                    .send(ShouldUpdateStatus(WaitingForBroker))
                    .await;
                self.initial_info = Some(info.clone());
            }
            Failed(_) => {
                info!("Trying to recover from failure in 6s...");
                sleep(Duration::from_secs(6)).await;
                self.recover_from_failure(rt_tx.clone()).await;
            }
            _ => {}
        }
        self.last_info = Some(info);
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
        let best_broker_mgmt_url = np.mgmt_addr.clone();
        let public_port = self.network_status.as_ref().unwrap();
        let public_port = public_port.public_rpc_port.as_ref().unwrap().clone() as u16;

        let fa = &CONFIG.local_worker().forwarder_bind_addresses;
        let fa = fa.first().unwrap();
        let fa: Vec<&str> = fa.split(":").collect();
        let fap = fa.get(1).unwrap().to_string();
        let fa = fa.get(0).unwrap().to_string();
        let fa = if fa.eq("0.0.0.0") {
            my_ipv4_interfaces().first().unwrap().ip.to_string()
        } else {
            fa
        };
        let address_string = format!("http://{}:{}", fa, fap);

        let keepalive_content = LocalWorkerIdentity {
            instance_name: np.instance_name.clone(),
            instance_id: np.id.clone(),
            address_string,
            public_port,
            public_key: self.public_key.as_ref().unwrap().to_string(),
        };
        drop(np);

        self.best_broker = Some(peer.clone());
        self.best_broker_name = Some(name.clone());
        self.best_broker_mgmt_url = Some(best_broker_mgmt_url);
        self.keepalive_content = Some(serde_json::to_string(&keepalive_content).unwrap());

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
                // if !(self.peer_browser_started) {
                tokio::spawn(start_peer_browser(&self.mdns, tx.clone(), &RT_CTX));
                self.peer_browser_started = true;
                // }
                sleep(Duration::from_secs(3)).await;
                tokio::spawn(wait_for_broker_peer(rt_tx.clone()));
            }
            PendingProvision => {
                stop_broker_browse(self.mdns).expect("Failed to stop broker browsing");
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
                    .send(ShouldUpdateStatus(Failed(WorkerRuntimeFailReason {
                        broker: None,
                        pr: Some(msg),
                    })))
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
            WaitingForBroker => {
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
                        if loop_num >= 5 {
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

pub fn stop_broker_browse(mdns: &ServiceDaemon) -> Result<()> {
    let result = mdns.stop_browse(SERVICE_PSN_BROKER);
    match result {
        Ok(_) => Ok(()),
        Err(err) => match err {
            Error::Again => stop_broker_browse(mdns),
            _ => Err(anyhow::Error::from(err)),
        },
    }
}

pub async fn check_pruntime_health(rt_tx: WorkerRuntimeChannelMessageSender) {
    info!("Starting pinging pRuntime for health check...");
    loop {
        let wr = WR.read().await;
        let duration = Duration::from_secs(match wr.status {
            Starting => 1,
            WaitingForBroker => 15,
            PendingProvision => 15,
            Started => 6,
            Failed(_) => 3,
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

pub async fn check_current_broker_health_loop(rt_tx: WorkerRuntimeChannelMessageSender) {
    loop {
        let wr = WR.read().await;
        let status = wr.status.clone();
        drop(wr);

        match status {
            PendingProvision => {
                check_current_broker_health(rt_tx.clone()).await;
            }
            Started => {
                check_current_broker_health(rt_tx.clone()).await;
            }
            _ => {}
        };

        sleep(Duration::from_millis(match status {
            PendingProvision => LOCAL_WORKER_KEEPALIVE_INTERVAL,
            Started => LOCAL_WORKER_KEEPALIVE_INTERVAL,
            _ => 3000,
        }))
        .await;
    }
}

pub async fn check_current_broker_health(rt_tx: WorkerRuntimeChannelMessageSender) {
    let wr = WR.read().await;
    let broker = wr.best_broker.as_ref().unwrap();
    let broker = broker.lock().await;
    let mut url = broker.mgmt_addr.clone();
    let body = (&wr).keepalive_content.as_ref().unwrap().to_string();
    drop(broker);
    drop(wr);
    url.push_str(R_V0_LOCAL_WORKER_KEEPALIVE);
    let res = (&REQ_CLIENT)
        .put(url)
        .header(CONTENT_TYPE, CONTENT_TYPE_JSON)
        .body(body)
        .send()
        .await;
    match res {
        Ok(res) => {
            let res = res.json::<MyIdentity>().await;
            match res {
                Ok(res) => {
                    debug!("Broker responding keepalive: {:?}", res)
                }
                Err(err) => {
                    error!("Broker failed on processing keepalive: {}", &err);
                    let _ = rt_tx
                        .clone()
                        .send(ShouldSetBrokerFailed(format!("{}", err)))
                        .await;
                }
            }
        }
        Err(err) => {
            error!("Failed to send keepalive to current broker: {}", &err);
            let _ = rt_tx
                .clone()
                .send(ShouldSetBrokerFailed(format!("{}", err)))
                .await;
        }
    };
}

pub async fn start_peer_browser(
    mdns: &ServiceDaemon,
    tx: BrokerPeerUpdateSender,
    ctx: &AsyncRuntimeContext,
) {
    let pm = &RT_CTX.peer_manager;
    crate::register_service(mdns, &ctx).await;
    pm.browse_brokers(mdns, tx.clone(), &RT_CTX).await;
    debug!("Broker browsing stopped.");
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
