use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::utils::serde::deser_fromstr;

#[derive(Debug, Clone, Copy)]
pub enum PriceSource {
    MempoolSpace,
    CoinGecko,
}

impl std::fmt::Display for PriceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PriceSource::MempoolSpace => write!(f, "mempool.space"),
            PriceSource::CoinGecko => write!(f, "coingecko"),
        }
    }
}

impl FromStr for PriceSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "mempool.space" => Ok(PriceSource::MempoolSpace),
            "coingecko" => Ok(PriceSource::CoinGecko),
            _ => Err("Invalid price source".to_string()),
        }
    }
}

impl<'de> Deserialize<'de> for PriceSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deser_fromstr(deserializer)
    }
}

impl Serialize for PriceSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self)
    }
}
