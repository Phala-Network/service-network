use super::CONFIG;
use anyhow::{anyhow, Context, Result};
use fast_socks5::server::{Config as Socks5Config, Socks5Server, Socks5Socket};
use fast_socks5::util::target_addr::TargetAddr;
use fast_socks5::{Result as SocksResult, SocksError};
use futures::future::join_all;
use futures::StreamExt;
use lazy_static::lazy_static;
use log::{debug, info};
use service_network::config::BrokerConfig;
use service_network::runtime::AsyncRuntimeContext;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use trust_dns_resolver::config::{NameServerConfigGroup, ResolverConfig, ResolverOpts};

use trust_dns_resolver::TokioAsyncResolver;

type WrappedResolver = Arc<Mutex<TokioAsyncResolver>>;

lazy_static! {
    pub static ref RESOLVER: WrappedResolver = {
        let BrokerConfig {
            doh_server_port,
            doh_server_addresses,
            doh_server_dns_name,
            ..
        } = CONFIG.broker().clone();
        let addrs: Vec<_> = doh_server_addresses
            .iter()
            .map(|s| IpAddr::from_str(s).expect("Invalid DNS over HTTPS server!"))
            .collect();
        let addrs = addrs.as_slice();

        let resolver_config = ResolverConfig::from_parts(
            None,
            Vec::new(),
            NameServerConfigGroup::from_ips_https(
                addrs,
                doh_server_port,
                doh_server_dns_name,
                true,
            ),
        );
        let resolver = TokioAsyncResolver::tokio(resolver_config, ResolverOpts::default()).unwrap();
        Arc::new(Mutex::new(resolver))
    };
}

pub async fn start(ctx: &AsyncRuntimeContext) {
    let config = &ctx.config;
    let config = config.broker();

    let addresses = &config.outbound_bind_addresses;
    let listeners: Vec<_> = addresses
        .iter()
        .map(|addr| {
            let addr = addr.to_string();
            spawn_server(addr)
        })
        .collect();
    join_all(listeners).await;
}

fn spawn_server(addr: String) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut socks_config = Socks5Config::default();
        socks_config.set_udp_support(false);
        socks_config.set_dns_resolve(false);
        socks_config.set_transfer_data(false);
        let mut listener = Socks5Server::bind(addr.clone())
            .await
            .expect(&format!("Failed to bind on {}", addr.clone()));
        listener.set_config(socks_config);

        info!("Listening on {} for outbound socks...", &addr);

        let mut incoming = listener.incoming();
        while let Some(socket_res) = incoming.next().await {
            match socket_res {
                Ok(socket) => {
                    tokio::spawn(async {
                        if let Err(err) = handle_socket(socket).await {
                            debug!("socket handle error = {:?}", err);
                        }
                    });
                }
                Err(err) => {
                    debug!("accept error = {:?}", err);
                }
            };
        }
    })
}

async fn handle_socket<T>(socket: Socks5Socket<T>) -> SocksResult<()>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    let mut socks5_socket = socket
        .upgrade_to_socks5()
        .await
        .context("Failed to upgrade incoming socket to socks5.")?;

    let raw_target_addr = socks5_socket
        .target_addr()
        .context("Failed to get target address from incoming socket.")?;

    let resolved_target_addr = match raw_target_addr {
        TargetAddr::Ip(addr) => *addr,
        TargetAddr::Domain(domain_name, port) => {
            // TODO: check for local peers and i2p addresses
            let addr = nslookup(domain_name.to_string())
                .await
                .context("Failed to resolve from internet")?;
            SocketAddr::new(addr, *port)
        }
    };
    let resolved_target_addr = match check_bogon(resolved_target_addr) {
        Ok(addr) => addr,
        Err(_) => return Ok(()),
    };

    let mut remote_stream = TcpStream::connect(resolved_target_addr).await?;

    match tokio::io::copy_bidirectional(&mut remote_stream, &mut socks5_socket).await {
        Ok(res) => {
            debug!("socket transfer closed ({}, {})", res.0, res.1);
            Ok(())
        }
        Err(err) => match err.kind() {
            ErrorKind::NotConnected => {
                debug!("socket transfer closed by client");
                Ok(())
            }
            ErrorKind::ConnectionReset => {
                debug!("socket transfer closed by downstream");
                Ok(())
            }
            _ => Err(SocksError::Other(anyhow!(
                "socket transfer error: {:#}",
                err
            ))),
        },
    }
}

fn check_bogon(addr: SocketAddr) -> Result<SocketAddr> {
    // TODO: check bogon addresses
    Ok(addr)
}

async fn nslookup(domain_name: String) -> Result<IpAddr> {
    let resolver = RESOLVER.lock().await;
    let addr = resolver
        .lookup_ip(domain_name)
        .await
        .context("Failed to resolve name.")?;
    drop(resolver);
    let addr = addr
        .iter()
        .filter_map(|r| match r {
            IpAddr::V4(_a) => Some(r),
            IpAddr::V6(_a) => None, // TODO: Support IPV6
        })
        .collect::<Vec<_>>();
    if addr.len() <= 0 {
        return Err(anyhow!("Name not resolved."));
    }
    Ok(*addr.last().unwrap())
}
