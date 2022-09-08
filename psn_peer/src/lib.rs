use std::sync::{Arc, RwLock};

pub mod runtime;

#[derive(Clone, Debug)]
pub enum PeerRole {
    PrUndefined = 0,
    PrLocalWorker = 1,
    PrRemoteWorker = 2,
    PrBroker = 3,
}

#[derive(Debug)]
pub struct PeerConfig {
    pub role: PeerRole,
}

pub type WrappedPeerConfig = Arc<RwLock<PeerConfig>>;
