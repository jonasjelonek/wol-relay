use std::thread::JoinHandle;
use std::sync::mpsc;
use std::io::ErrorKind;
use std::fmt::Debug;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use pnet::datalink::{
    self, 
    Channel, 
    Config, 
    DataLinkSender, 
    NetworkInterface
};
use pnet::packet::{
    ethernet::EthernetPacket,
    Packet
};
use pnet::util::MacAddr;
use tokio_util::sync::CancellationToken;

use crate::common;

pub const ETHERTYPE_WOL: u16 = 0x0842;

#[derive(Debug, Default)]
pub struct Layer2Config {
    pub interfaces: Vec<String>,
}

struct WolMessage<'a> {
    iface: NetworkInterface,
    pkt: EthernetPacket<'a>,
    target: MacAddr,
}

fn l2_wol_check(pkt: &EthernetPacket) -> bool {
    pkt.get_ethertype().0 == ETHERTYPE_WOL && 
        pkt.get_destination().is_broadcast() &&
        crate::common::check_wol_payload(pkt.payload())
}

pub fn l2_worker(cfg: Layer2Config, token: CancellationToken) -> Result<Vec<JoinHandle<()>>> {
    let interfaces: Vec<NetworkInterface> = pnet::datalink::interfaces()
        .into_iter()
        .filter(|iface| cfg.interfaces.contains(&iface.name))
        .collect();
    if interfaces.len() == 0 {
        log::error!("no suitable L2 interface available!");
        return Err(anyhow!("no suitable L2 interfaces available!"));
    }

    let mut handles: Vec<JoinHandle<()>> = Vec::new();
    let mut senders: Vec<(u32, Box<dyn DataLinkSender>)> = Vec::new();
    let (mpsc_tx, mpsc_rx) = mpsc::sync_channel::<WolMessage>(8);
    
    let mut dl_cfg = Config::default();
    dl_cfg.read_timeout = Some(Duration::from_millis(50));
    dl_cfg.write_timeout = Some(Duration::from_millis(500));

    for iface in interfaces {
        if iface.is_loopback() || !iface.is_up() { continue; }

        let (tx, mut rx) = match datalink::channel(&iface, dl_cfg) {
            Ok(Channel::Ethernet(tx, rx )) => (tx, rx),
            Ok(_) | Err(_) => continue,
        };
        log::debug!("listening on interface '{}'", iface.name);

        senders.push((iface.index, tx));

        let mpsc_tx = mpsc_tx.clone();
        let token = token.clone();
        let iface = iface.clone();

        /* 
         * Not using tasks here. Due to the amount of incoming Ethernet frames, this can lead to the issue that
         * the TX tasks consume most of the time and the RX task won't run unless the TX tasks yield control
         * (e.g. when the mpsc's buffer is full and then send blocks).
         */
        let h = std::thread::spawn(move || {
            loop {
                if token.is_cancelled() { log::trace!("[listener][{}] exit", iface.name); break; }

                let packet = match rx.next() {
                    Ok(pkt) => pkt,
                    Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::WouldBlock => continue,
                    Err(_) => break,
                };

                match EthernetPacket::new(packet) {
                    Some(eth_pkt) => {
                        log::trace!("[listener][{}] ethernet packet from {} to {} of type {}",
                            iface.name, eth_pkt.get_source(), eth_pkt.get_destination(),
                            eth_pkt.get_ethertype());

                        if !l2_wol_check(&eth_pkt) { continue; }

                        log::debug!("[listener][{}] received WakeOnLan Ethernet packet", iface.name);

                        let pkt = EthernetPacket::owned(eth_pkt.packet().to_vec()).unwrap();
                        mpsc_tx.send(WolMessage { 
                            iface: iface.clone(),
                            pkt,
                            target: common::wol_payload_get_target_mac(eth_pkt.payload()),
                        }).ok();
                    },
                    None => continue,
                }
            }
        });

        handles.push(h);
    }

    let token = token.clone();
    let h = std::thread::spawn(move || {
        let mut cooldown_list: HashMap<MacAddr, Instant> = HashMap::new();

        loop {
            if token.is_cancelled() { log::trace!("[relay] exit"); break; }

            let wol_msg = match mpsc_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(msg) => msg,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            
            if let Some(t) = cooldown_list.get(&wol_msg.target) {
                if t.elapsed() < common::COOLDOWN_DUR {
                    continue;
                } else {
                    cooldown_list.remove(&wol_msg.target);
                }
            }

            log::debug!("[relay] relaying WakeOnLan packet from {}", wol_msg.pkt.get_source());

            for (if_idx, sender) in senders.iter_mut() {
                if *if_idx != wol_msg.iface.index {
                    sender.send_to(wol_msg.pkt.packet(), None);
                }
            }
        }
    });

    handles.push(h);
    Ok(handles)
}