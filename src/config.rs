use std::{fmt::Debug, net::Ipv4Addr};
use std::net::IpAddr;

use serde::{Deserialize, Deserializer};

/// Deserializes an absent field as None and an unset field as T::default. 
/// 
/// This avoid having Option<Option<T>> as in serde_with::rust::double_option
pub fn deserialize_absent_or_null<'de, D, T: Default>(deserializer: D) -> Result<Option<T>, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(Option::deserialize(deserializer)?.or(Some(T::default())))
}


#[derive(Debug, Deserialize, Default)]
pub struct Layer2Config {
    interfaces: Vec<String>,
}

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

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default, deserialize_with = "deserialize_absent_or_null")]
    pub layer2: Option<Layer2Config>,
    
    #[serde(default, deserialize_with = "deserialize_absent_or_null")]
    pub layer4: Option<Layer4Config>,
}
