use crate::mgmt::BrokerMgmtShared;
use crate::LocalWorkerManagerChannelMessage::ReceivedKeepAlive;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{extract::Extension, Json};
use service_network::mgmt_types::LocalWorkerIdentity;
use std::sync::Arc;

pub async fn handle_keepalive(
    Extension(shared): Extension<Arc<BrokerMgmtShared>>,
    Json(lwi): Json<LocalWorkerIdentity>,
) -> impl IntoResponse {
    let _ = shared.tx.clone().send(ReceivedKeepAlive(lwi)).await;
    (StatusCode::IM_A_TEAPOT, shared.my_id.clone())
}
