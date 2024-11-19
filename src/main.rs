use std::thread::JoinHandle;

use pnet::datalink::NetworkInterface;

use tokio_util::sync::CancellationToken;

use clap::Parser;

mod common;
mod layer2;
mod layer4;

#[derive(Parser)]
struct Cli {
    #[arg(long, default_value_t = false)]
    l2: bool,

    #[arg(long)]
    l2_exclude_if: Vec<String>,
}

fn main() {
    let opts = Cli::parse();
    let cancel_token = CancellationToken::new();
	let sigint_token = cancel_token.clone();

    ctrlc::set_handler(move || {
		println!("Received SIGINT");
		sigint_token.cancel();
	}).expect("Failed to install SIGINT handler");

    let interfaces: Vec<NetworkInterface> = pnet::datalink::interfaces()
        .into_iter()
        .filter(|iface| !opts.l2_exclude_if.contains(&iface.name))
        .collect();

    let mut l2_handles: Vec<JoinHandle<()>> = Vec::new();
    if opts.l2 {
        l2_handles.extend(layer2::l2_worker(&interfaces[..], cancel_token));
    }

    layer4::l4_worker();

    // wait for workers
    l2_handles.into_iter().for_each(|h| { let _ = h.join(); });
}