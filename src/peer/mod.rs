pub mod broker;
pub mod local_worker;

use crate::config::{PeerConfig, PeerRole};
use crate::peer::broker::BrokerPeerManager;
use crate::runtime::AsyncRuntimeContext;
use if_addrs::{IfAddr, Ifv4Addr};
use local_worker::{BrokerPeerUpdateSender, LocalPeerWorkerManager};
use log::trace;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use std::collections::HashMap;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};

pub const SERVICE_PSN_LOCAL_WORKER: &str = "_psn-worker._tcp.local.";
pub const SERVICE_PSN_BROKER: &str = "_psn-broker._tcp.local.";
pub const SERVICE_LOCAL_DOMAIN: &str = "local.";

pub const BROWSE_LOOP_SLEEP_DURATION: Duration = Duration::from_secs(300);
pub const BROWSE_SEARCH_INTERVAL: Duration = Duration::from_secs(120);
pub const BROWSE_RESOLVE_TIMEOUT: Duration = Duration::from_secs(30);

pub type TxtMap = HashMap<String, String>;
pub type WrappedPeerManager = Arc<RwLock<PeerManager>>;
pub type PeerStatusChangeCb = Box<dyn Fn(PeerStatus) + Send + Sync + 'static>;

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
                ServiceEvent::SearchStopped(_) => return,
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
