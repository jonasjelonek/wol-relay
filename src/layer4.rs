use std::{fmt::Debug, net::Ipv4Addr};
use std::net::IpAddr;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Layer4Config {
    listen_addr: IpAddr,
    listen_port: u16,
}

impl Default for Layer4Config {
    fn default() -> Self {
        Self {
            listen_addr: IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            listen_port: 9,
        }
    }
}

pub fn l4_worker(cfg: Layer4Config) {

}