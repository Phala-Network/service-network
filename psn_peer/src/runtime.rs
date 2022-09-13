use crate::config::{PeerConfig, PeerRole};
use crate::peer::{PeerManager, WrappedPeerManager};
use std::sync::Arc;
use tokio::sync::RwLock;

pub type WrappedAsyncRuntimeContext = Arc<RwLock<AsyncRuntimeContext>>;

pub struct AsyncRuntimeContext {
    pub config: PeerConfig,
    pub peer_manager: WrappedPeerManager,
}

impl AsyncRuntimeContext {
    pub fn new(config: PeerConfig) -> Self {
        let peer_manager = match config.role {
            PeerRole::PrUndefined => {
                panic!("Unsupported role!")
            }
            PeerRole::PrLocalWorker => PeerManager::init_for_local_worker(),
            PeerRole::PrRemoteWorker => {
                panic!("Unsupported role!")
            }
            PeerRole::PrBroker => PeerManager::init_for_broker(),
        };
        let peer_manager = Arc::new(RwLock::new(peer_manager));
        AsyncRuntimeContext {
            config,
            peer_manager,
        }
    }
    pub fn new_wrapped(config: PeerConfig) -> WrappedAsyncRuntimeContext {
        Arc::new(RwLock::new(Self::new(config)))
    }
}
