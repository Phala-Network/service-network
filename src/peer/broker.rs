use crate::peer::PeerStatus;
use anyhow::{anyhow, Context, Result};
use futures::future::join_all;
use log::trace;
use mdns_sd::ServiceInfo;
use std::collections::BTreeMap;
use std::fmt::{Debug, Formatter};

use crate::peer::local_worker::{BrokerPeerUpdateSender, WrappedBrokerPeer};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::runtime::AsyncRuntimeContext;

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
    pub async fn update_peer_with_service_info(
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
        let mgmt_addr = format!("http://{}:{:?}", mgmt_addr, mgmt_port);

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

        Ok(())
    }

    pub async fn verify_best_instance(&mut self) -> Result<Option<WrappedBrokerPeer>> {
        if let Some(b) = self.get_best_instance().await {
            let b = b.clone();
            let bb = b.lock().await;
            let oa = bb.service_info.get_properties();
            let _ = oa.get("oa").context("Invalid outbound address").unwrap();
            self.best_instance_name = Some(bb.instance_name.to_string());
            return Ok(Some(b.clone()));
        };
        Ok(None)
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
        self.status = s;
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

#[derive(Debug, Clone)]
pub enum BrokerPeerUpdate {
    None,
    PeerStatusChanged(String, PeerStatus),
    BestPeerChanged(String, String),
}
