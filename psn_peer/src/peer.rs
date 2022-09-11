use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use if_addrs::{IfAddr, Ifv4Addr};
use log::{debug, info, trace};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use tokio::sync::RwLock;

use crate::config::PeerRole;
use crate::runtime::{WrappedAsyncRuntimeContext, WrappedRuntime};

pub const SERVICE_PSN_LOCAL_WORKER: &'static str = "_psn-local-worker._tcp.local.";
pub const SERVICE_PSN_BROKER: &'static str = "_psn-broker._tcp.local.";
pub const SERVICE_LOCAL_DOMAIN: &'static str = "local.";

pub const BROWSE_LOOP_SLEEP_DURATION: Duration = Duration::from_secs(300);
pub const BROWSE_SEARCH_INTERVAL: Duration = Duration::from_secs(120);
pub const BROWSE_RESOLVE_TIMEOUT: Duration = Duration::from_secs(30);

pub type TxtMap = HashMap<String, String>;
pub type WrappedPeerManager = Arc<RwLock<PeerManager>>;

#[derive(Debug)]
pub struct PeerManager {
    pub my_role: PeerRole,
    pub broker_peer_map: BTreeMap<String, BrokerPeer>,
    pub local_worker_map: BTreeMap<String, LocalWorkerPeer>,
}

impl PeerManager {
    fn init(my_role: PeerRole) -> Self {
        let broker_peer_map = BTreeMap::new();
        let local_worker_map = BTreeMap::new();
        Self {
            my_role,
            broker_peer_map,
            local_worker_map,
        }
    }

    pub fn init_for_broker() -> Self {
        Self::init(PeerRole::PrBroker)
    }

    pub fn init_for_local_worker() -> Self {
        Self::init(PeerRole::PrLocalWorker)
    }

    pub async fn browse_local_workers(ctx_w: WrappedAsyncRuntimeContext, _rt_w: WrappedRuntime) {
        let ctx_w_r = &ctx_w.clone();

        let mdns = ServiceDaemon::new().expect("Failed to create daemon");
        let receiver = mdns
            .browse(SERVICE_PSN_LOCAL_WORKER)
            .expect("Failed to browse");

        while let Ok(event) = receiver.recv_async().await {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    PeerManager::update_local_worker_peer(ctx_w_r.clone(), info).await;
                }
                other_event => {
                    trace!("[app] Received other event: {:?}", &other_event);
                }
            }
        }
    }

    async fn update_local_worker_peer(ctx: WrappedAsyncRuntimeContext, info: ServiceInfo) {}

    pub async fn browse_brokers(ctx_w: WrappedAsyncRuntimeContext, _rt_w: WrappedRuntime) {
        let ctx_w_r = &ctx_w.clone();

        let mdns = ServiceDaemon::new().expect("Failed to create daemon");
        let receiver = mdns.browse(SERVICE_PSN_BROKER).expect("Failed to browse");

        while let Ok(event) = receiver.recv_async().await {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    PeerManager::update_broker_peer(ctx_w_r.clone(), info).await;
                }
                other_event => {
                    trace!("[browse_brokers] Received other event: {:?}", &other_event);
                }
            }
        }
    }

    async fn update_broker_peer(ctx: WrappedAsyncRuntimeContext, service_info: ServiceInfo) {
        trace!("[update_broker_peer] Resolved a new service: {service_info:?}");
        // if service.domain != SERVICE_LOCAL_DOMAIN {
        //     return;
        // }
        // // println!(
        // //     "Service {}{:?}@{:?} (type {:?})\t\t[{:?}]",
        // //     "-", service.service_name, service.domain, service.reg_type, service
        // // );
        // let resolve = service.resolve().timeout(BROWSE_RESOLVE_TIMEOUT);
        // let service = &service;
        // let r = resolve.try_filter_map(move |r| async move {
        //     let txt = TxtRecord::parse(&r.txt).map(|rdata| {
        //         rdata
        //             .iter()
        //             .map(|(key, value)| {
        //                 (
        //                     String::from(String::from_utf8_lossy(key)),
        //                     value.map(|value| String::from(String::from_utf8_lossy(value))),
        //                 )
        //             })
        //             .collect::<Vec<_>>()
        //     });
        //     let txt = txt.unwrap();
        //     Ok(Some(txt))
        // });
        // let txt: Result<Vec<_>, Error> = r.try_collect().await;
        // let txt: _ = txt.unwrap();
        //
        // if txt.len() > 0 {
        //     let ctx = ctx.read().await;
        //     let pm = &ctx.peer_manager.clone();
        //     let mut pm = pm.write().await;
        //     let pm = &mut pm.broker_peer_map;
        //     let _ = pm.insert(service.service_name.to_string(), BrokerPeer {});
        //     println!("{pm:?}");
        // }
        // println!("{txt:?}");
    }
}

#[derive(Debug)]
pub struct LocalWorkerPeer {}

impl LocalWorkerPeer {}

#[derive(Debug)]
pub struct BrokerPeer {}

impl BrokerPeer {}

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
