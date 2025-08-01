use serde::{de, Deserialize, Deserializer, Serialize, Serializer};
use std::str::FromStr;

pub const PRICE_UPDATE_INTERVAL: u64 = 300; // seconds

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Currency {
    #[default]
    USD,
    GBP,
    EUR,
}

impl std::fmt::Display for Currency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Currency::USD => write!(f, "USD"),
            Currency::GBP => write!(f, "GBP"),
            Currency::EUR => write!(f, "EUR"),
        }
    }
}

impl FromStr for Currency {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "USD" => Ok(Currency::USD),
            "GBP" => Ok(Currency::GBP),
            "EUR" => Ok(Currency::EUR),
            _ => Err(()),
        }
    }
}

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(|_| de::Error::custom(format!("invalid currency: {}", s)))
    }
}

impl Serialize for Currency {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PriceSetting {
    pub currency: Currency,
    pub is_enabled: bool,
}

impl PriceSetting {
    pub fn is_valid(&self) -> bool {
        matches!(self.currency, Currency::USD | Currency::GBP | Currency::EUR)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PriceSource {
    MempoolSpace,
    CoinGecko,
}

impl std::fmt::Display for PriceSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PriceSource::MempoolSpace => write!(f, "mempool.space"),
            PriceSource::CoinGecko => write!(f, "CoinGecko"),
        }
    }
}
impl FromStr for PriceSource {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mempool.space" => Ok(PriceSource::MempoolSpace),
            "CoinGecko" => Ok(PriceSource::CoinGecko),
            _ => Err(()),
        }
    }
}
impl<'de> Deserialize<'de> for PriceSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        FromStr::from_str(&s).map_err(|_| de::Error::custom(format!("invalid price source: {}", s)))
    }
}

impl Serialize for PriceSource {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

// impl PriceSource {
//     pub fn supports_currency(&self, currency: &Currency) -> bool {
//         // FIXME: This is a placeholder implementation.
//         match self {
//             PriceSource::MempoolSpace => matches!(currency, Currency::USD | Currency::EUR),
//             PriceSource::CoinGecko => matches!(currency, Currency::USD | Currency::GBP | Currency::EUR),
//         }
//     }
// }

// impl Default for Setting {
//     fn default() -> Self {
//         Self {
//             fiat_currency: Currency::USD,
//             is_enabled: true,
//         }
//     }
// }

#[derive(Debug, Clone)]
pub struct Price {
    pub value: f64,
    pub currency: Currency,
    pub timestamp: u64,
    pub source: PriceSource,
}

pub async fn get_fiat_price(currency: Currency, source: PriceSource) -> Result<Price, String> {
    Ok(Price {
        value: 0.0,
        currency,
        timestamp: 0,
        source,
    })
}
