use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalWorkerIdentity {
    pub instance_name: String,
    pub instance_id: String,
    pub address_string: String,
    pub public_port: u16,
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MyIdentity {
    pub instance_name: String,
    pub instance_id: String,
}

pub static R_V0_LOCAL_WORKER_KEEPALIVE: &'static str = "/v0/local_worker/keepalive";
