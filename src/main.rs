use std::{net::SocketAddr, thread};

use layer2::Layer2Config;
use layer4::Layer4Config;
use log::LevelFilter;
use pnet::ipnetwork::IpNetwork;
use simple_logger::SimpleLogger;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use clap::Parser;

mod common;
mod layer2;
mod layer4;

#[derive(Parser)]
struct Cli {
    #[arg(short, long, default_value_t = LevelFilter::Info)]
    log: LevelFilter,

    #[arg(long)]
    l2: bool,

    #[arg(long)]
    l2_if: Vec<String>,

    #[arg(long)]
    l4: bool,

    #[arg(long)]
    l4_listen_on: Vec<SocketAddr>,
    
    #[arg(long)]
    l4_relay_to: Vec<IpNetwork>,
}

#[tokio::main]
async fn main() {
    let opts = Cli::parse();

    SimpleLogger::new()
        .with_level(opts.log)
        .env()
        .init()
        .unwrap();

    let cancel_token = CancellationToken::new();
	let sigint_token = cancel_token.clone();

    ctrlc::set_handler(move || {
		log::info!("Interrupted by SIGINT");
		sigint_token.cancel();
	}).expect("Failed to install SIGINT handler");

    let mut l2_handles: Vec<thread::JoinHandle<()>> = Vec::new();
    if opts.l2 {
        let l2_cfg = Layer2Config { interfaces: opts.l2_if };
        l2_handles.extend(layer2::l2_worker(l2_cfg, cancel_token.clone()));
    }

    let mut l4_handles: Option<JoinSet<()>> = None;
    if opts.l4 {
        let l4_cfg = Layer4Config {
            listen_on: opts.l4_listen_on,
            relay_to: opts.l4_relay_to,
        };
        l4_handles = Some(layer4::l4_worker(l4_cfg, cancel_token));
    }

    // wait for workers
    l2_handles.into_iter().for_each(|h| { let _ = h.join(); });
    if let Some(tasks) = l4_handles {
        tasks.join_all().await;
    }
}