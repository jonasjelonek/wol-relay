use std::fmt::Debug;

use serde::{Deserialize, Deserializer};

use crate::{
    layer2::Layer2Config,
    layer4::Layer4Config
};

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


#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default, deserialize_with = "deserialize_absent_or_null")]
    pub layer2: Option<Layer2Config>,
    
    #[serde(default, deserialize_with = "deserialize_absent_or_null")]
    pub layer4: Option<Layer4Config>,
}
