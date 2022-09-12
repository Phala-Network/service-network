use psn_peer::runtime::{WrappedAsyncRuntimeContext, WrappedRuntime};

async fn start(ctx_w: WrappedAsyncRuntimeContext, rt_w: WrappedRuntime) {
    let ctx = ctx_w.clone();
    let ctx = ctx.read().await;
    let config = &ctx.config.clone();
    let config = config.read().await;
    let config = config.broker.as_deref().unwrap();

    let port = config.outbound_socks_port;
}
