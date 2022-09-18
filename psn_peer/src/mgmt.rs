use crate::config::PeerConfig;
use axum::{extract::Extension, routing::any, Router};
use hyper::Server;
use std::net::SocketAddr;
use std::sync::Arc;

pub fn create_router() -> Router {
    let router = Router::new();
    let router = router.route("/ping", any(messages::keepalive));

    router
}

pub async fn start_with_router(
    bind_addr: &SocketAddr,
    router: Router,
    config: &'static PeerConfig,
) {
    let router = router.layer(Extension(Arc::new(config.clone())));
    Server::bind(bind_addr)
        .serve(router.into_make_service())
        .await
        .unwrap();
}

pub mod messages {
    use crate::config::PeerConfig;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::{extract::Extension, Json};
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    #[derive(Serialize, Deserialize)]
    struct Keepalive {
        instance_name: String,
        instance_id: String,
    }

    pub async fn keepalive(Extension(config): Extension<Arc<PeerConfig>>) -> impl IntoResponse {
        (
            StatusCode::IM_A_TEAPOT,
            Json(Keepalive {
                instance_name: config.common.instance_name.to_string(),
                instance_id: config.instance_id.to_string(),
            }),
        )
    }
}
