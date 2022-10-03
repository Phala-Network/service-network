use anyhow::{anyhow, Context, Result};
use futures::future::join_all;
use if_addrs::{IfAddr, Ifv4Addr};
use log::{trace, warn};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Debug, Formatter};

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver as mpsc__Receiver, Sender as mpsc__Sender};
use tokio::sync::{Mutex, RwLock};

use crate::config::{PeerConfig, PeerRole};
use crate::peer::BrokerPeerUpdate::{BestPeerChanged, PeerStatusChanged};
use crate::runtime::AsyncRuntimeContext;

pub const SERVICE_PSN_LOCAL_WORKER: &'static str = "_psn-worker._tcp.local.";
pub const SERVICE_PSN_BROKER: &'static str = "_psn-broker._tcp.local.";
pub const SERVICE_LOCAL_DOMAIN: &'static str = "local.";

pub const BROWSE_LOOP_SLEEP_DURATION: Duration = Duration::from_secs(300);
pub const BROWSE_SEARCH_INTERVAL: Duration = Duration::from_secs(120);
pub const BROWSE_RESOLVE_TIMEOUT: Duration = Duration::from_secs(30);

pub type TxtMap = HashMap<String, String>;
pub type WrappedPeerManager = Arc<RwLock<PeerManager>>;
pub type WrappedBrokerPeer = Arc<Mutex<BrokerPeer>>;
pub type BrokerPeerUpdateSender = mpsc__Sender<BrokerPeerUpdate>;
pub type BrokerPeerUpdateReceiver = mpsc__Receiver<BrokerPeerUpdate>;
pub type PeerStatusChangeCb = Box<dyn Fn(PeerStatus) + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub enum BrokerPeerUpdate {
    None,
    PeerStatusChanged(String, PeerStatus),
    BestPeerChanged(String, String),
}

#[derive(Debug)]
pub struct BrokerPeerManager {
    pub map: BTreeMap<String, Arc<Mutex<BrokerPeer>>>,
    pub best_instance_name: Option<String>,
}
impl BrokerPeerManager {
    pub fn new() -> Self {
        let map = BTreeMap::new();
        Self {
            map,
            best_instance_name: None,
        }
    }
    async fn update_peer_with_service_info(
        &mut self,
        service_info: ServiceInfo,
        tx: BrokerPeerUpdateSender,
        _ctx: &AsyncRuntimeContext,
    ) -> Result<()> {
        trace!("Trying to update broker with service info: {service_info:?}");
        let props = &service_info.get_properties();
        let instance_name = props.get("in").context("Invalid instance name").unwrap();
        if instance_name.len() <= 0 {
            return Err(anyhow!("Invalid instance name"));
        }
        let id = props
            .get("i")
            .context("Unexpected empty instance id")
            .unwrap();
        if id.len() <= 0 {
            return Err(anyhow!("Unexpected empty instance id"));
        }
        let cost = props.get("c").context("Invalid cost").unwrap();
        let cost = atoi::atoi::<u8>(cost.as_bytes());
        if cost.is_none() {
            return Err(anyhow!(
                "Peer candidate {}:{} has invalid cost",
                instance_name,
                id
            ));
        }
        let cost = cost.unwrap();
        let mgmt_port = service_info.get_port();
        let mgmt_addr = service_info.get_addresses();
        let mgmt_addr = mgmt_addr.iter().next().context("Invalid address").unwrap();
        let mgmt_addr = mgmt_addr.to_string();
        let mgmt_addr = format!("http://{}:{}", mgmt_addr, mgmt_port);

        if self.map.contains_key(instance_name) {
            let _peer = self.map.get(instance_name).unwrap();
            // TODO: warn for id change
            return Ok(());
        }

        let map = &mut self.map;

        let mut new_peer = BrokerPeer::new(
            cost,
            instance_name,
            id,
            service_info.clone(),
            mgmt_addr,
            tx.clone(),
        );
        new_peer.start_life_check().await;
        let new_peer = Arc::new(Mutex::new(new_peer));
        map.insert(instance_name.to_string(), new_peer.clone());

        self.set_best_instance(tx.clone()).await;

        Ok(())
    }

    fn get_cached_best(&self) -> Option<Arc<Mutex<BrokerPeer>>> {
        match &self.best_instance_name {
            Some(n) => Some(self.map.get(n).unwrap().clone()),
            None => None,
        }
    }

    async fn set_best_instance(&mut self, tx: BrokerPeerUpdateSender) -> Result<()> {
        if let Some(b) = self.get_best_instance().await {
            let b = b.clone();
            let bb = b.lock().await;
            let oa = bb.service_info.get_properties();
            let oa = oa.get("oa").context("Invalid outbound address").unwrap();
            self.best_instance_name = Some(bb.instance_name.to_string());
            if let Err(e) = tx
                .send(BestPeerChanged(
                    bb.instance_name.to_string(),
                    format!("socks5://{}", oa),
                ))
                .await
            {
                warn!("[set_best_instance] Failed: {:?}", e)
            };
        };
        Ok(())
    }

    async fn get_best_instance(&self) -> Option<WrappedBrokerPeer> {
        let v = self.map.values();
        if v.len() <= 0 {
            return None;
        }
        let v: Vec<_> = v
            .map(|p| async {
                let p = p.clone();
                let p = p.lock().await;
                let n = p.instance_name.to_string();
                (n, p.cost)
            })
            .collect();
        let mut v: Vec<(String, u8)> = join_all(v).await;
        v.sort_by(|a, b| a.1.cmp(&b.1));
        let v: Vec<_> = v
            .into_iter()
            .map(|(n, _cost)| self.map.get(n.as_str()))
            .collect();
        Some(v[0].unwrap().clone())
    }
}

pub trait PeerLifecycle {
    fn set_status(&mut self, s: PeerStatus) -> ();
}

pub struct BrokerPeer {
    pub cost: u8,
    pub service_info: ServiceInfo,
    pub instance_name: String,
    pub id: String,
    pub status: PeerStatus,
    pub mgmt_addr: String,
    pub tx: BrokerPeerUpdateSender,
}
impl BrokerPeer {
    pub fn new(
        cost: u8,
        instance_name: &str,
        id: &str,
        service_info: ServiceInfo,
        mgmt_addr: String,
        tx: BrokerPeerUpdateSender,
    ) -> Self {
        Self {
            cost,
            instance_name: instance_name.to_string(),
            id: id.to_string(),
            service_info,
            mgmt_addr,
            status: PeerStatus::Init,
            tx,
        }
    }
    async fn set_status(&mut self, s: PeerStatus) {
        let ns = s.clone();
        self.status = s;
        let tx = &self.tx.clone();
        tx.clone()
            .send(PeerStatusChanged(self.instance_name.to_string(), ns))
            .await
            .expect("Failed to send PeerStatusChanged")
    }
    async fn start_life_check(&mut self) {
        self.set_status(PeerStatus::Start).await;
    }
}
impl Debug for BrokerPeer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {:?}", self.instance_name, self.service_info)
    }
}

#[derive(Debug)]
pub struct LocalPeerWorkerManager {
    map: BTreeMap<String, LocalWorkerPeer>,
}
impl LocalPeerWorkerManager {
    pub fn new() -> Self {
        let map = BTreeMap::new();
        Self { map }
    }
}

#[derive(Debug, Clone)]
pub struct LocalWorkerPeer {}
impl LocalWorkerPeer {}

#[derive(Debug)]
pub struct PeerManager {
    pub my_role: PeerRole,
    pub broker: Arc<Mutex<BrokerPeerManager>>,
    pub local_worker: Arc<Mutex<LocalPeerWorkerManager>>,
}
impl PeerManager {
    // We'll maintain keepalive connections to peers instead of relying on the existence of DNS-SD records for that there are TTLs of 75 minutes for non-host records per RFC6762

    fn init(my_role: PeerRole) -> Self {
        let broker = BrokerPeerManager::new();
        let broker = Arc::new(Mutex::new(broker));
        let local_worker = LocalPeerWorkerManager::new();
        let local_worker = Arc::new(Mutex::new(local_worker));
        Self {
            my_role,
            broker,
            local_worker,
        }
    }

    pub fn init_for_broker(_config: &PeerConfig) -> Self {
        Self::init(PeerRole::PrBroker(None))
    }

    pub fn init_for_local_worker(_config: &PeerConfig) -> Self {
        Self::init(PeerRole::PrLocalWorker(None))
    }

    pub async fn browse_local_workers(&self, mdns: &ServiceDaemon, ctx: &AsyncRuntimeContext) {
        let receiver = mdns
            .browse(SERVICE_PSN_LOCAL_WORKER)
            .expect("Failed to browse");

        while let Ok(event) = receiver.recv_async().await {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    PeerManager::update_local_worker_peer(ctx, info).await;
                }
                other_event => {
                    trace!("[browse_local_workers] {:?}", &other_event);
                }
            }
        }
    }

    async fn update_local_worker_peer(_ctx: &AsyncRuntimeContext, service_info: ServiceInfo) {
        trace!("[update_local_worker_peer] Trying to update local worker with service info: {service_info:?}");
        // TODO: local routing table for local workers
    }

    pub async fn browse_brokers(
        &self,
        mdns: &ServiceDaemon,
        tx: BrokerPeerUpdateSender,
        ctx: &AsyncRuntimeContext,
    ) {
        let receiver = mdns.browse(SERVICE_PSN_BROKER).expect("Failed to browse");

        while let Ok(event) = receiver.recv_async().await {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let broker = &self.broker.clone();
                    let mut broker = broker.lock().await;
                    let tx = tx.clone();
                    let _ = broker.update_peer_with_service_info(info, tx, ctx).await;
                    drop(broker);
                }
                other_event => {
                    trace!("[browse_brokers] {:?}", &other_event);
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum PeerStatus {
    Init,
    Start,
    Alive,
    Aiding,
    Dead,
}

pub fn my_ipv4_interfaces() -> Vec<Ifv4Addr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| {
            if i.is_loopback() {
                None
            } else {
                match i.addr {
                    IfAddr::V4(ifv4) => Some(ifv4),
                    _ => None,
                }
            }
        })
        .collect()
}
