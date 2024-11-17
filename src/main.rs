use std::net;
use std::thread::JoinHandle;
use std::sync::mpsc;

use pnet::datalink::{self, Channel, Config, DataLinkSender, NetworkInterface};
use pnet::packet::ethernet::EthernetPacket;
use pnet::packet::Packet;

use tokio_util::sync::CancellationToken;

use clap::Parser;

const ETHERTYPE_WOL: u16 = 0x0842;
const ETHERTYPE_IP4: u16 = 0x0800;
const ETHERTYPE_IP6: u16 = 0x86DD;

const BROADCAST_MAC: [u8; 6] = [ 0xff, 0xff, 0xff, 0xff, 0xff, 0xff ];

#[derive(Parser)]
struct Cli {
    #[arg(short, long, default_value_t = false)]
    raw: bool
}

struct WolMessage<'a> {
    iface: NetworkInterface,
    pkt: EthernetPacket<'a>,
}

fn check_wol_payload(payload: &[u8]) -> bool {
    if payload.len() < 102 { return false; }

    let blocks: Vec<&[u8]> = payload.chunks(6).collect();
    if blocks[0] != BROADCAST_MAC {
        return false;
    }

    for i in 2..blocks.len() {
        if blocks[i] != blocks[1] {
            return false;
        }
    }

    true
}

fn eth_wol_check(pkt: &EthernetPacket) -> bool {
    pkt.get_ethertype().0 == ETHERTYPE_WOL && 
        pkt.get_destination().is_broadcast() &&
        check_wol_payload(pkt.payload())
}

fn l2_worker(interfaces: &[NetworkInterface], token: CancellationToken) -> Vec<JoinHandle<()>> {
    let mut handles: Vec<JoinHandle<()>> = Vec::new();
    let mut senders: Vec<(u32, Box<dyn DataLinkSender>)> = Vec::new();
    let (mpsc_tx, mpsc_rx) = mpsc::sync_channel::<WolMessage>(8);

    for iface in interfaces {
        if iface.is_loopback() || !iface.is_up() { continue; }
        println!("Creating listener for interface '{}'", iface.name);

        let Ok(Channel::Ethernet(tx, mut rx)) = datalink::channel(&iface, Config::default())
        else {
            println!("Unable to get ethernet channel on interface '{}'", iface.name);
            continue;
        };

        senders.push((iface.index, tx));

        let mpsc_tx = mpsc_tx.clone();
        let token = token.clone();
        let iface = iface.clone();

        /* 
         * Not using tasks here, they run in an event loop. Due to the amount of incoming Ethernet frames, this
         * can lead to the issue that the TX tasks consume most of the time and the RX task won't run unless the
         * TX tasks yield control (e.g. when the mpsc's buffer is full and then send blocks).
         */
        let h = std::thread::spawn(move || {
            while let Ok(packet) = rx.next() {
                if token.is_cancelled() { break; }

                match EthernetPacket::new(packet) {
                    Some(eth_pkt) => {
                        if !eth_wol_check(&eth_pkt) { continue; }

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
            if token.is_cancelled() { break; }

            let wol_msg = match mpsc_rx.recv() {
                Ok(msg) => msg,
                Err(_) => break,
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

fn l4_worker() {

}

fn main() {
    let opts = Cli::parse();
    let cancel_token: CancellationToken = CancellationToken::new();
	let sigint_token = cancel_token.clone();

    ctrlc::set_handler(move || {
		println!("Received SIGINT");
		sigint_token.cancel();
	}).expect("Failed to install SIGINT handler");

    let interfaces = pnet::datalink::interfaces();

    let mut l2_handles: Vec<JoinHandle<()>> = Vec::new();
    if opts.raw {
        l2_handles.extend(l2_worker(&interfaces[..], cancel_token));
    }

    l4_worker();

    // wait for workers
    l2_handles.into_iter().for_each(|h| { let _ = h.join(); });
}