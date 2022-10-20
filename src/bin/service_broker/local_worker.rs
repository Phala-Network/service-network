use crate::local_worker::KnownLocalWorkerStatus::*;
use crate::local_worker::LocalWorkerManagerChannelMessage::*;
use crate::LW_MAP;
use log::{debug, error, info, warn};
use service_network::config::{BROKER_PEER_DEAD_COUNT, BROKER_PEER_LOST_COUNT};
use service_network::mgmt_types::LocalWorkerIdentity;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::RwLock;
use urlparse::urlparse;

pub enum LocalWorkerManagerChannelMessage {
    ShouldCheckPeerHealth,
    ReceivedKeepAlive(LocalWorkerIdentity),
}

pub type LocalWorkerManagerChannelMessageSender = Sender<LocalWorkerManagerChannelMessage>;
pub type LocalWorkerManagerChannelMessageReceiver = Receiver<LocalWorkerManagerChannelMessage>;

pub type LocalWorkerMap = BTreeMap<String, KnownLocalWorker>;
pub type WrappedLocalWorkerMap = Arc<RwLock<LocalWorkerMap>>;

pub async fn local_worker_manager(
    _tx: LocalWorkerManagerChannelMessageSender,
    mut rx: LocalWorkerManagerChannelMessageReceiver,
) {
    let mut lw_vec_keys: Vec<String> = Vec::new();
    let lw_map_lock = LW_MAP.clone();

    loop {
        while let Some(msg) = rx.recv().await {
            match msg {
                ShouldCheckPeerHealth => {
                    check_peer_health((&lw_map_lock).clone(), &lw_vec_keys).await;
                }
                ReceivedKeepAlive(lw) => {
                    let lw_map = (&lw_map_lock).clone();
                    let mut lw_map = lw_map.write().await;
                    let key = lw.public_key.as_str();
                    let klw = lw_map.get_mut(key);
                    if let Some(mut klw) = klw {
                        match klw.status {
                            Dead => {
                                lw_vec_keys = create_local_worker(&mut lw_map, lw);
                            }
                            _ => {
                                update_local_worker(&mut klw, lw);
                            }
                        }
                    } else {
                        lw_vec_keys = create_local_worker(&mut lw_map, lw);
                    }
                    drop(lw_map);
                }
            }
        }
    }
}

fn create_local_worker(lw_map: &mut LocalWorkerMap, lw: LocalWorkerIdentity) -> Vec<String> {
    let LocalWorkerIdentity {
        instance_name,
        instance_id,
        address_string,
        public_key,
        public_port,
    } = lw;

    let key = public_key.clone();
    let uri = urlparse(address_string.as_str());
    let hostname = uri.hostname.unwrap();
    let forwarder_port = uri.port.unwrap();

    info!(
        "Hello new worker({}/{}).",
        instance_name.as_str(),
        public_key.as_str()
    );

    let ret = KnownLocalWorker {
        status: Active,
        hostname,
        forwarder_port,
        instance_name,
        instance_id,
        address_string,
        public_key,
        public_port,
        lost_count: 0,
        lost_mark: false,
    };
    debug!("KnownLocalWorker: {:?}", &ret);
    lw_map.insert(key, ret);
    lw_map
        .iter()
        .filter_map(|(key, val)| match val.status.clone() {
            Dead => None,
            _ => Some(key.clone()),
        })
        .collect::<Vec<String>>()
}

fn update_local_worker(klw: &mut KnownLocalWorker, lw: LocalWorkerIdentity) {
    let LocalWorkerIdentity {
        instance_name,
        instance_id,
        address_string,
        public_key,
        ..
    } = lw;

    if !(instance_id.eq(klw.instance_id.as_str())) {
        warn!(
            "Worker {} has changed instance id, it may have restarted.",
            instance_name.as_str()
        )
    }
    klw.instance_id = instance_id;

    if !(address_string.eq(klw.address_string.as_str())) {
        warn!(
            "Worker {} has changed its IP address, there may be IP address collisions.",
            instance_name.as_str()
        )
    }
    if !(public_key.eq(klw.public_key.as_str())) {
        error!(
            "[FATAL] Worker {} has changed public key, please check your network environment.",
            instance_name.as_str()
        )
    }

    match klw.status.clone() {
        Active => {
            klw.lost_mark = false;
            klw.lost_count = 0;
        }
        Lost => {
            klw.status = Active;
            klw.lost_count = 0;
            klw.lost_mark = false;
            info!(
                "Worker {} has recovered from lost state.",
                instance_name.as_str()
            )
        }
        Dead => {
            warn!(
                "[FATAL] This is a bug, `update_local_worker` ran for dead worker {}.",
                instance_name.as_str()
            )
        }
    }
}

async fn check_peer_health(lw_map_lock: WrappedLocalWorkerMap, lw_vec_keys: &Vec<String>) {
    let mut lw_map = lw_map_lock.write().await;
    let _ = lw_vec_keys.iter().for_each(|k| {
        let lw = lw_map.get_mut(k);
        if let Some(mut lw) = lw {
            match lw.status.clone() {
                Active => {
                    if lw.lost_mark && (lw.lost_count > BROKER_PEER_LOST_COUNT) {
                        lw.status = Lost;
                        warn!(
                            "Worker peer unreachable: {}/{}",
                            lw.instance_name, lw.public_key
                        );
                    }
                    lw.lost_mark = true;
                    lw.lost_count += 1;
                }
                Lost => {
                    if lw.lost_count > BROKER_PEER_DEAD_COUNT {
                        lw.status = Dead;
                        warn!("Worker peer dead: {}/{}", lw.instance_name, lw.public_key);
                    }
                    lw.lost_mark = true;
                    lw.lost_count += 1;
                }
                Dead => {
                    // ignored
                }
            }
        }
    });
    drop(lw_map);
}

#[derive(Clone, Debug)]
pub enum KnownLocalWorkerStatus {
    Active,
    Lost,
    Dead,
}

#[derive(Clone, Debug)]
pub struct KnownLocalWorker {
    pub status: KnownLocalWorkerStatus,
    pub hostname: String,
    pub forwarder_port: u16,
    pub address_string: String,
    pub public_port: u16,
    pub public_key: String,
    pub instance_name: String,
    pub instance_id: String,
    pub lost_count: u8,
    pub lost_mark: bool,
}

impl KnownLocalWorker {}
