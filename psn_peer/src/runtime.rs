use crate::config::{PeerConfig, PeerRole};
use crate::peer::{PeerManager};

use std::sync::Arc;
use tokio::sync::RwLock;

pub type WrappedAsyncRuntimeContext = Arc<RwLock<AsyncRuntimeContext>>;

#[derive(Debug)]
pub struct AsyncRuntimeContext {
    pub config: PeerConfig,
    pub peer_manager: PeerManager,
}

impl AsyncRuntimeContext {
    pub fn new(config: PeerConfig) -> Self {
        let peer_manager = match config.role {
            PeerRole::PrUndefined => {
                panic!("Unsupported role!")
            }
            PeerRole::PrLocalWorker(_) => PeerManager::init_for_local_worker(&config),
            PeerRole::PrRemoteWorker => {
                panic!("Unsupported role!")
            }
            PeerRole::PrBroker(_) => PeerManager::init_for_broker(&config),
        };
        AsyncRuntimeContext {
            config,
            peer_manager,
        }
    }
    pub fn new_wrapped(config: PeerConfig) -> WrappedAsyncRuntimeContext {
        Arc::new(RwLock::new(Self::new(config)))
    }
}
