use psn_peer::runtime::WrappedAsyncRuntimeContext;

async fn start(ctx: WrappedAsyncRuntimeContext) {
    let ctx = ctx.read().await;
    let config = &ctx.config;
    let config = config.broker.as_ref().unwrap();

    let _port = config.outbound_socks_port;
}
