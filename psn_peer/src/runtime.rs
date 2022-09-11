use crate::config::{PeerConfig, PeerRole, WrappedPeerConfig};
use crate::peer::{PeerManager, WrappedPeerManager};
use futures::future::try_join_all;
use futures::TryFuture;
use std::fmt::Debug;
use std::future::Future;
use std::sync::Arc;
use tokio::runtime::Runtime;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

pub type WrappedAsyncRuntimeContext = Arc<RwLock<AsyncRuntimeContext>>;
pub type WrappedRuntime = Arc<RwLock<Runtime>>;

pub struct AsyncRuntimeContext {
    pub config: WrappedPeerConfig,
    pub peer_manager: WrappedPeerManager,
}

impl AsyncRuntimeContext {
    pub fn init(config: PeerConfig) -> WrappedAsyncRuntimeContext {
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
        let config = Arc::new(RwLock::new(config));
        Arc::new(RwLock::new(AsyncRuntimeContext {
            config,
            peer_manager,
        }))
    }

    // pub fn get_peer_manager(&self) -> &'static WrappedPeerManager {
    //     let pm = &self.peer_manager;
    //     &pm.clone()
    // }

    pub fn spawn<T, F>(
        ctx_w: WrappedAsyncRuntimeContext,
        rt_w: WrappedRuntime,
        f: F,
    ) -> JoinHandle<<T as Future>::Output>
    where
        T: Future + Send + 'static,
        T::Output: Send + 'static,
        F: FnOnce(WrappedAsyncRuntimeContext, WrappedRuntime) -> T + Send + 'static,
    {
        let rt = rt_w.try_read().unwrap();
        let future = f(ctx_w.clone(), rt_w.clone());
        let handle = rt.spawn(future);
        drop(rt);

        handle
    }

    pub fn block_on_all<I>(rt_w: WrappedRuntime, iter: I) -> ()
    where
        I: IntoIterator,
        I::Item: TryFuture,
        <<I as IntoIterator>::Item as TryFuture>::Error: Debug,
    {
        let rt = rt_w.try_read().unwrap();
        rt.block_on(try_join_all(iter)).expect("block_on_all");
    }
}
