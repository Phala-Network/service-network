use crate::peer::{SERVICE_PSN_BROKER, SERVICE_PSN_LOCAL_WORKER};
use rand::distributions::{Alphanumeric, DistString};
use serde::Deserialize;

#[derive(Clone, Debug)]
pub enum PeerRole {
    PrUndefined,
    PrLocalWorker(Option<LocalWorkerConfig>),
    PrRemoteWorker,
    PrBroker(Option<BrokerConfig>),
}

#[derive(Debug)]
pub struct PeerConfig {
    pub role: PeerRole,
    pub version: String,
    pub git_revision: String,
    pub instance_id: String,
    pub common: CommonConfig,
}

impl PeerConfig {
    pub fn host_name(&self) -> String {
        let role = match self.role {
            PeerRole::PrLocalWorker(_) => "local_worker",
            PeerRole::PrRemoteWorker => "remote_worker",
            PeerRole::PrBroker(_) => "broker",
            _ => panic!("Invalid app role!"),
        };
        format!(
            "{}.{}.{}.psn.local",
            self.instance_id,
            self.common.instance_name.as_str(),
            role
        )
    }

    pub fn mdns_fullname(&self) -> String {
        let root = match self.role {
            PeerRole::PrLocalWorker(_) => SERVICE_PSN_LOCAL_WORKER,
            PeerRole::PrBroker(_) => SERVICE_PSN_BROKER,
            _ => panic!("Invalid app role!"),
        }
        .to_string();
        format!("{}.{}", self.common.instance_name.as_str(), root)
    }

    pub fn build_from_env(role: PeerRole, version: String, git_revision: String) -> Self {
        let instance_id = Alphanumeric.sample_string(&mut rand::thread_rng(), 16);

        let common = match envy::prefixed("PSN_").from_env::<CommonConfig>() {
            Ok(config) => config,
            Err(error) => panic!("Error while parsing common config: {:#?}", error),
        };

        let role_config = match role {
            PeerRole::PrBroker(_) => match envy::prefixed("PSN_BROKER_").from_env::<BrokerConfig>() {
                Ok(config) => PeerRole::PrBroker(Some(config)),
                Err(error) => panic!("Error while parsing broker config: {:#?}", error),
            },
            PeerRole::PrLocalWorker(_) =>
                match envy::prefixed("PSN_LW_").from_env::<LocalWorkerConfig>() {
                    Ok(config) => PeerRole::PrLocalWorker(Some(config)),
                    Err(error) => panic!("Error while parsing local worker config: {:#?}", error),
                },
            _ => panic!("Invalid role!"),
        };

        PeerConfig {
            version,
            git_revision,
            instance_id,
            common,
            role: role_config,
        }
    }

    pub fn broker(&self) -> &BrokerConfig {
        match &self.role {
            PeerRole::PrBroker(config) => config.as_ref().unwrap(),
            _ => panic!("Current peer is not a broker!")
        }
    }

    pub fn local_worker(&self) -> &LocalWorkerConfig {
        match &self.role {
            PeerRole::PrLocalWorker(config) => config.as_ref().unwrap(),
            _ => panic!("Current peer is not a broker!")
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct CommonConfig {
    #[serde(default = "generate_instance_name")]
    pub instance_name: String,
    #[serde(default = "default_mgmt_port")]
    pub mgmt_port: u16,
}

#[derive(Deserialize, Clone, Debug)]
pub struct BrokerConfig {
    #[serde(default = "default_outbound_socks_port")]
    pub outbound_socks_port: u16,
    #[serde(default = "default_inbound_http_server_bind_address")]
    pub inbound_http_server_bind_address: Vec<String>,
    pub inbound_http_server_accessible_address_prefix: String,
    #[serde(default = "default_cost")]
    pub cost: u8,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LocalWorkerConfig {
    #[serde(default = "default_pruntime_address")]
    pub pruntime_address: String,
    #[serde(default = "default_forwarder_socks_port")]
    pub forwarder_socks_port: u16,
}

fn generate_instance_name() -> String {
    let hostname = hostname::get().unwrap();
    let hostname = hostname.to_str().unwrap().split(".");
    let hostname: Vec<&str> = hostname.collect();
    let hostname = hostname[0].to_string();
    hostname
}

fn default_mgmt_port() -> u16 {
    1919
}

fn default_outbound_socks_port() -> u16 {
    1981
}

fn default_inbound_http_server_bind_address() -> Vec<String> {
    vec!["0.0.0.0:19810".to_string()]
}

fn default_cost() -> u8 {
    10
}

fn default_pruntime_address() -> String {
    "http://127.0.0.1:8080".to_string()
}

fn default_forwarder_socks_port() -> u16 {
    1982
}
