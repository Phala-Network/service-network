use crate::mgmt::{BrokerMgmtShared, MyIdentity};
use crate::LocalWorkerManagerChannelMessage::ReceivedKeepAlive;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{extract::Extension, Json};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Serialize, Deserialize)]
pub struct LocalWorkerIdentity {
    pub instance_name: String,
    pub instance_id: String,
    pub address_string: String,
    pub public_port: u16,
    pub public_key: String,
}

pub async fn handle_keepalive(
    Extension(shared): Extension<Arc<BrokerMgmtShared>>,
    Json(lwi): Json<LocalWorkerIdentity>,
) -> impl IntoResponse {
    let config = &shared.config;
    let _ = shared.tx.clone().send(ReceivedKeepAlive(lwi.clone())).await;
    (
        StatusCode::IM_A_TEAPOT,
        Json(MyIdentity {
            instance_name: config.common.instance_name.to_string(),
            instance_id: config.instance_id.to_string(),
        }),
    )
}
