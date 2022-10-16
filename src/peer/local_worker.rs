use crate::peer::broker::{BrokerPeer, BrokerPeerUpdate};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver as mpsc__Receiver, Sender as mpsc__Sender};
use tokio::sync::Mutex;

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

pub type WrappedBrokerPeer = Arc<Mutex<BrokerPeer>>;
pub type BrokerPeerUpdateSender = mpsc__Sender<BrokerPeerUpdate>;
pub type BrokerPeerUpdateReceiver = mpsc__Receiver<BrokerPeerUpdate>;
