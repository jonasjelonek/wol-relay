use std::{path::PathBuf, thread::JoinHandle};

use log::LevelFilter;
use simple_logger::SimpleLogger;
use tokio_util::sync::CancellationToken;

use clap::Parser;

mod common;
mod config;
mod layer2;
mod layer4;

#[derive(Parser)]
struct Cli {
    #[arg(short, long)]
    config_file: PathBuf,

    #[arg(short, long, default_value_t = LevelFilter::Info)]
    log: LevelFilter,
}

fn main() {
    let opts = Cli::parse();

    SimpleLogger::new()
        .with_level(opts.log)
        .env()
        .init()
        .unwrap();

    let cfg_path = PathBuf
        ::from(shellexpand::tilde(&opts.config_file.to_string_lossy()).into_owned())
        .canonicalize()
        .expect("Invalid config file path specified");
    let cfg_str = std::fs::read_to_string(cfg_path).unwrap();
    let cfg: config::Config = serde_yml::from_str(&cfg_str).unwrap();

    let cancel_token = CancellationToken::new();
	let sigint_token = cancel_token.clone();

    ctrlc::set_handler(move || {
		log::info!("Interrupted by SIGINT");
		sigint_token.cancel();
	}).expect("Failed to install SIGINT handler");

    let mut l2_handles: Vec<JoinHandle<()>> = Vec::new();
    if let Some(l2_cfg) = cfg.layer2 {
        l2_handles.extend(layer2::l2_worker(l2_cfg, cancel_token));
    }

    if let Some(l4_cfg) = cfg.layer4 {
        layer4::l4_worker(l4_cfg);
    }

    // wait for workers
    l2_handles.into_iter().for_each(|h| { let _ = h.join(); });
}