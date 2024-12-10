use std::collections::HashMap;
use std::time::{Duration, Instant};
use std::{fmt::Debug, net::Ipv4Addr};
use std::net::{IpAddr, SocketAddr, SocketAddrV4};

use pnet::ipnetwork::{
    IpNetwork,
    Ipv4Network,
    IpNetworkError
};
use pnet::util::MacAddr;
use serde::Deserialize;

use tokio::{
    net::UdpSocket,
    sync::mpsc,
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::common;

#[derive(Debug, Deserialize)]
pub struct Layer4Config {
    listen_on: Vec<SocketAddr>,
    relay_to: Vec<IpNetwork>,
}

impl Default for Layer4Config {
    fn default() -> Self {
        Self {
            listen_on: vec![SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 9))],
            relay_to: vec![IpNetwork::V4(Ipv4Network::new(Ipv4Addr::UNSPECIFIED, 0).unwrap())]
        }
    }
}

const IPV4_PRIVATE_A: Result<Ipv4Network, IpNetworkError> = Ipv4Network::new(Ipv4Addr::new(10, 0, 0, 0), 8);
const IPV4_PRIVATE_B: Result<Ipv4Network, IpNetworkError> = Ipv4Network::new(Ipv4Addr::new(172, 16, 0, 0), 12);
const IPV4_PRIVATE_C: Result<Ipv4Network, IpNetworkError> = Ipv4Network::new(Ipv4Addr::new(192, 168, 0, 0), 16);
const IPV4_UNSPEC: IpAddr = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

struct WolMessage {
    src: SocketAddr,
    target: MacAddr,
    msg: Box<[u8]>,
}

fn is_private_network(net: &IpNetwork) -> bool {
    match net {
        IpNetwork::V4(v4net) => {
            v4net.is_subnet_of(IPV4_PRIVATE_A.unwrap()) ||
                v4net.is_subnet_of(IPV4_PRIVATE_B.unwrap()) ||
                v4net.is_subnet_of(IPV4_PRIVATE_C.unwrap())
        },
        IpNetwork::V6(_) => unreachable!(),
    }
}

fn sanitize_destination_networks(mut relay_to: Vec<IpNetwork>) -> Vec<IpNetwork> {
    let networks_avail = pnet::datalink::interfaces()
        .into_iter()
        .flat_map(|e| e.ips)
        .filter_map(|net| {
            (net.is_ipv4() && is_private_network(&net))
                .then_some(IpNetwork::new(net.network(), net.prefix()).unwrap())
        })
        .collect::<Vec<IpNetwork>>();
    
    if networks_avail.len() == 0 { panic!("No available networks!") }
    if relay_to.len() == 0 { panic!("No relay networks specified!") }

    relay_to = relay_to
        .into_iter()
        .map(|net| IpNetwork::new(net.network(), net.prefix()).unwrap())
        .collect();
    relay_to.sort();
    relay_to.dedup();

    let mut networks: Vec<IpNetwork>;
    if relay_to[0].ip().is_unspecified() {
        networks = networks_avail;
    } else {
        networks = Vec::new();
        for net in relay_to {
            if !networks_avail.contains(&net) {
                log::warn!("network {} is not available!", net);
            }

            networks.push(net);
        }
    }

    // There can be duplicates in some cases. 
    networks.sort();
    networks.dedup();
    networks
}

pub fn l4_worker(cfg: Layer4Config, token: CancellationToken) -> JoinSet<()> {
    let (mpsc_tx, mut mpsc_rx) = mpsc::channel::<WolMessage>(8);
    let mut tasks: JoinSet<()> = JoinSet::new();

    // CHECK: what's less worse, a new Vec or sorting the Vec?
    //let mut listen_on = cfg.listen_on;
    //listen_on.sort();
    //if listen_on[0].ip() == IPV4_UNSPEC {
    //    listen_on.truncate(1);
    //}
    let listen_on = match cfg.listen_on.iter().find(|s| s.ip() == IPV4_UNSPEC) {
        Some(addr) => vec![addr.clone()],
        None => cfg.listen_on,
    };
    let networks = sanitize_destination_networks(cfg.relay_to);

    for addr in listen_on {
        let mpsc_tx = mpsc_tx.clone();
        let token = token.clone();

        tasks.spawn(async move {
            let sock = UdpSocket::bind(addr).await.unwrap();
            log::debug!("listening on socket '{}'", addr);

            loop {
                if token.is_cancelled() { break; }

                let mut buf: [u8; 128] = [0; 128];
                let (len, from) = match tokio::time::timeout(
                    Duration::from_millis(100), 
                    sock.recv_from(&mut buf)
                ).await {
                    Ok(res) => res.unwrap(),
                    Err(_) => continue,
                };

                log::trace!("received message from {}", from);
                if !common::check_wol_payload(&buf[..len]) {
                    continue;
                }
                log::debug!("received WOL message from {}", from);

                mpsc_tx.send(WolMessage {
                    src: from,
                    target: common::wol_payload_get_target_mac(&buf[..len]),
                    msg: Box::from(&buf[..len])
                }).await.unwrap();
            }
        });
    }

    log::debug!("Relaying to {} networks", networks.len());
    for net in networks.iter() {
        log::debug!("relay to network: {}", net);
    }

    tasks.spawn(async move {
        let sock = UdpSocket::bind(
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))
        ).await.unwrap();
        sock.set_broadcast(true).unwrap();

        let mut cooldown_list: HashMap<MacAddr, Instant> = HashMap::new();
        
        loop {
            if token.is_cancelled() { break; }

            let msg = match tokio::time::timeout(
                Duration::from_millis(50), 
                mpsc_rx.recv()
            ).await {
                Ok(Some(m)) => m,
                Ok(_) => break,
                Err(_) => continue,
            };

            if let Some(t) = cooldown_list.get(&msg.target) {
                if t.elapsed() < common::COOLDOWN_DUR {
                    continue;
                } else {
                    cooldown_list.remove(&msg.target);
                }
            }

            log::debug!("relay message from {} to networks", msg.src);

            for net in networks.iter() {
                log::trace!("relaying message from {} to {}", msg.src, net);
                sock.send_to(
                    &msg.msg, 
                    SocketAddr::new(net.broadcast(), 9)
                ).await.unwrap();
            }

            cooldown_list.insert(msg.target, Instant::now());
        }
    });

    tasks
}
