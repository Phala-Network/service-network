use crate::PeerConfig;
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
    pub config: Arc<RwLock<PeerConfig>>,
}

impl AsyncRuntimeContext {
    pub fn init(config: PeerConfig) -> WrappedAsyncRuntimeContext {
        Arc::new(RwLock::new(AsyncRuntimeContext {
            config: Arc::new(RwLock::new(config)),
        }))
    }

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
        let future = f(ctx_w, rt_w.clone());
        let rt = rt_w.try_read().unwrap();
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

pub fn init(config: PeerConfig) -> WrappedAsyncRuntimeContext {
    AsyncRuntimeContext::init(config).clone()
}
