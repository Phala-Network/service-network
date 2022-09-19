mod pb;

use anyhow::Result;
use env_logger::{Builder as LoggerBuilder, Target};
use log::{debug, info, warn};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use psn_peer::config::{PeerConfig, PeerRole};
use psn_peer::peer::{
    my_ipv4_interfaces, BrokerPeerUpdate, BrokerPeerUpdateReceiver,
    BrokerPeerUpdateSender,
    SERVICE_PSN_LOCAL_WORKER,
};
use psn_peer::runtime::AsyncRuntimeContext;
use std::collections::HashMap;
use std::net::Ipv4Addr;


#[macro_use]
extern crate lazy_static;

lazy_static! {
    pub static ref VERSION: String = env!("CARGO_PKG_VERSION").to_string();
    pub static ref GIT_REVISION: String = option_env!("PROJECT_GIT_REVISION")
        .unwrap_or("dev")
        .to_string();
    pub static ref CONFIG: PeerConfig = PeerConfig::build_from_env(
        PeerRole::PrLocalWorker(None),
        VERSION.to_string(),
        GIT_REVISION.to_string(),
    );
    pub static ref RT_CTX: AsyncRuntimeContext = AsyncRuntimeContext::new(CONFIG.clone());
}

#[tokio::main]
async fn main() {
    let mut builder = LoggerBuilder::from_default_env();
    builder.target(Target::Stdout);
    builder.init();

    info!(
        "pSN local worker version {}-{}",
        VERSION.as_str(),
        GIT_REVISION.as_str()
    );
    debug!("Staring local worker broker with config: {:?}", &*CONFIG);

    let (update_best_peer_tx, update_best_peer_rx) = tokio::sync::mpsc::channel(32);

    let _ = tokio::join!(
        tokio::spawn(handle_update_best_peer(update_best_peer_rx, CONFIG.clone())),
        tokio::spawn(local_worker(update_best_peer_tx.clone(), &RT_CTX)),
    );
}

async fn local_worker(tx: BrokerPeerUpdateSender, ctx: &AsyncRuntimeContext) {
    let mdns = ServiceDaemon::new().expect("Failed to create daemon");
    let pm = &RT_CTX.peer_manager;
    register_service(&mdns, &ctx).await;
    pm.browse_brokers(&mdns, tx.clone(), &RT_CTX).await;
}

async fn set_pruntime_network_with_peer(socks_url: String, _config: PeerConfig) -> Result<()> {
    debug!(
        "Trying to set pRuntime outbound with {}",
        socks_url.as_str()
    );

    Ok(())
}

async fn handle_update_best_peer(mut rx: BrokerPeerUpdateReceiver, config: PeerConfig) {
    while let Some(u) = rx.recv().await {
        match u {
            BrokerPeerUpdate::PeerStatusChanged(name, status) => {
                info!("Broker {} changed its status to {:?}", name, status);
            }
            BrokerPeerUpdate::BestPeerChanged(instance_name, socks_url) => {
                // todo: needs to be throttled
                match tokio::spawn(set_pruntime_network_with_peer(
                    socks_url.to_string(),
                    config.clone(),
                ))
                .await
                {
                    Ok(_) => {
                        info!(
                            "Updated pRuntime networking with ({}):{}",
                            instance_name.as_str(),
                            socks_url.as_str()
                        );
                    }
                    Err(err) => {
                        warn!("Failed to set pRuntime networking: {:?}", err);
                    }
                }
            }
            _ => {}
        }
    }
}

async fn register_service(mdns: &ServiceDaemon, _ctx: &AsyncRuntimeContext) {
    let common_config = &CONFIG.common;
    let _worker_config = CONFIG.local_worker();

    let my_addrs: Vec<Ipv4Addr> = my_ipv4_interfaces().iter().map(|i| i.ip).collect();

    // in => instance name
    // i => instance_id
    // mp => management port
    let service_info = ServiceInfo::new(
        SERVICE_PSN_LOCAL_WORKER,
        common_config.instance_name.as_str(),
        CONFIG.host_name().as_str(),
        &my_addrs[..],
        common_config.mgmt_port,
        Some(HashMap::from([
            ("in".to_string(), format!("{}", common_config.instance_name)),
            ("i".to_string(), format!("{}", CONFIG.instance_id)),
            ("mp".to_string(), format!("{}", common_config.mgmt_port)),
            // todo: worker info
        ])),
    )
    .expect("Failed to build service info");
    mdns.register(service_info.clone())
        .expect("Failed to register mDNS service");
    info!(
        "[register_service] Registered service for {}: {:?}",
        CONFIG.mdns_fullname(),
        &service_info
    );
}
