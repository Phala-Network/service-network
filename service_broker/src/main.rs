use psn_peer::runtime::{
    init as init_async_runtime, AsyncRuntimeContext, WrappedAsyncRuntimeContext, WrappedRuntime,
};
use psn_peer::{PeerConfig, PeerRole};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::runtime::Builder;
use tokio::sync::RwLock;

fn main() {
    let config = PeerConfig {
        role: PeerRole::PrBroker,
    };

    let rt_w = Arc::new(RwLock::new(
        Builder::new_multi_thread().enable_all().build().unwrap(),
    ));

    let rt = rt_w.try_read().unwrap();
    let _guard = rt.enter();
    drop(rt);

    let ctx_wrapper = init_async_runtime(config);
    // let ctx = ctx_wrapper.read().unwrap();
    // drop(ctx);
    // let ctx_wrapper = ctx_wrapper.clone();

    AsyncRuntimeContext::block_on_all(
        rt_w.clone(),
        vec![
            AsyncRuntimeContext::spawn(
                ctx_wrapper.clone(),
                rt_w.clone(),
                Box::new(move |cw, r| tcp_server(cw, r, "0.0.0.0:11451".to_owned())),
            ),
            AsyncRuntimeContext::spawn(
                ctx_wrapper.clone(),
                rt_w.clone(),
                Box::new(move |cw, r| tcp_server(cw, r, "0.0.0.0:11452".to_owned())),
            ),
        ],
    );
}

async fn tcp_server(_ctx_w: WrappedAsyncRuntimeContext, rt_w: WrappedRuntime, addr: String) {
    println!("Listening on {}", addr);
    let listener = TcpListener::bind(addr).await.unwrap();

    loop {
        let (mut socket, _) = listener.accept().await.unwrap();
        let rt = rt_w.read().await;
        rt.spawn(async move {
            let mut ret: Vec<u8> = Vec::new();

            loop {
                ret.clear();
                let n = socket
                    .read_buf(&mut ret)
                    .await
                    .expect("failed to read data from socket");

                if n == 0 {
                    return;
                }

                println!("n = {}, r = {:?}", n, ret);
                socket.write_all(&ret).await.expect("failed to write.");
            }
        });
        drop(rt);
        // drop(ctx);
    }
}
