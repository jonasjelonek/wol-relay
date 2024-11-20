use std::thread::JoinHandle;
use std::sync::mpsc;
use std::time::Duration;
use std::io::ErrorKind;
use std::fmt::Debug;

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
use tokio_util::sync::CancellationToken;
use serde::Deserialize;

pub const ETHERTYPE_WOL: u16 = 0x0842;

#[derive(Debug, Deserialize, Default)]
pub struct Layer2Config {
    pub interfaces: Vec<String>,
}

struct WolMessage<'a> {
    iface: NetworkInterface,
    pkt: EthernetPacket<'a>,
}

fn l2_wol_check(pkt: &EthernetPacket) -> bool {
    pkt.get_ethertype().0 == ETHERTYPE_WOL && 
        pkt.get_destination().is_broadcast() &&
        crate::common::check_wol_payload(pkt.payload())
}

pub fn l2_worker(cfg: Layer2Config, token: CancellationToken) -> Vec<JoinHandle<()>> {
    let interfaces: Vec<NetworkInterface> = pnet::datalink::interfaces()
        .into_iter()
        .filter(|iface| cfg.interfaces.contains(&iface.name))
        .collect();

    let mut handles: Vec<JoinHandle<()>> = Vec::new();
    let mut senders: Vec<(u32, Box<dyn DataLinkSender>)> = Vec::new();
    let (mpsc_tx, mpsc_rx) = mpsc::sync_channel::<WolMessage>(8);
    
    let mut dl_cfg = Config::default();
    dl_cfg.read_timeout = Some(Duration::from_millis(50));
    dl_cfg.write_timeout = Some(Duration::from_millis(500));

    for iface in interfaces {
        if iface.is_loopback() || !iface.is_up() { continue; }
        println!("Creating listener for interface '{}'", iface.name);

        let (tx, mut rx) = match datalink::channel(&iface, dl_cfg) {
            Ok(Channel::Ethernet(tx, rx )) => (tx, rx),
            Ok(_) | Err(_) => continue,
        };

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
                if token.is_cancelled() { println!("exit {}", iface.name); break; }

                let packet = match rx.next() {
                    Ok(pkt) => pkt,
                    Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::WouldBlock => continue,
                    Err(_) => break,
                };

                match EthernetPacket::new(packet) {
                    Some(eth_pkt) => {
                        if !l2_wol_check(&eth_pkt) { continue; }

                        println!("Received Ethernet WOL frame on interface '{}'", iface.name);

                        let pkt = EthernetPacket::owned(eth_pkt.packet().to_vec()).unwrap();
                        mpsc_tx.send(WolMessage { iface: iface.clone(), pkt }).unwrap();
                    },
                    None => continue,
                }
            }
        });

        handles.push(h);
    }

    let token = token.clone();
    let h = std::thread::spawn(move || {
        loop {
            if token.is_cancelled() { println!("exit rx"); break; }

            let wol_msg = match mpsc_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(msg) => msg,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            
            println!("Received a WOL packet from {}", wol_msg.pkt.get_source());

            for (if_idx, sender) in senders.iter_mut() {
                if *if_idx != wol_msg.iface.index {
                    sender.send_to(wol_msg.pkt.packet(), None);
                }
            }
        }
    });

    handles.push(h);
    handles
}