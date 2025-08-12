use std::str::FromStr;

use crate::fiat::api::{GetPriceResult, ListCurrenciesResult, PriceApiError};

use super::Currency;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PriceSource {
    #[default]
    CoinGecko,
    MempoolSpace,
}

/// All variants of `PriceSource`.
pub const ALL_PRICE_SOURCES: [PriceSource; 2] = [PriceSource::MempoolSpace, PriceSource::CoinGecko];

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

// #[derive(Debug)]
// pub enum ParseDataError {
//     UnexpectedType,
//     MissingKey(String),
// }

impl PriceSource {
    pub fn get_price_url(&self, currency: Currency) -> String {
        match self {
            PriceSource::MempoolSpace => "https://mempool.space/api/v1/prices".to_string(),
            PriceSource::CoinGecko => format!(
                "https://api.coingecko.com/api/v3/simple/price?symbols=btc&vs_currencies={}&include_last_updated_at=true",
                currency
            ),
        }
    }

    pub fn list_currencies_url(&self) -> String {
        match self {
            PriceSource::MempoolSpace => "https://mempool.space/api/v1/prices".to_string(),
            PriceSource::CoinGecko => {
                "https://api.coingecko.com/api/v3/simple/supported_vs_currencies".to_string()
            }
        }
    }

    pub fn parse_price_data(
        &self,
        currency: Currency,
        data: &serde_json::Value,
    ) -> Result<GetPriceResult, PriceApiError> {
        let (value, updated_at) = match self {
            PriceSource::MempoolSpace => {
                let value = data
                    .get(currency.to_string())
                    .and_then(|curr| curr.as_u64())
                    .ok_or(PriceApiError::CannotParseData("price".to_string()))?;
                let updated_at = data.get("timestamp").and_then(|t| t.as_u64());
                (value, updated_at)
            }
            PriceSource::CoinGecko => {
                let btc = data.get("btc").ok_or(PriceApiError::CannotParseData(
                    "missing key 'btc'".to_string(),
                ))?;
                let value = btc
                    .get(currency.to_string().to_lowercase())
                    .and_then(|curr| curr.as_u64())
                    .ok_or(PriceApiError::CannotParseData("price".to_string()))?;
                let updated_at = btc.get("last_updated_at").and_then(|t| t.as_u64());
                (value, updated_at)
            }
        };
        Ok(GetPriceResult { value, updated_at })
    }

    pub fn parse_currencies_data(
        &self,
        data: &serde_json::Value,
    ) -> Result<ListCurrenciesResult, PriceApiError> {
        let currencies: Vec<_> = data
            .as_object()
            .ok_or(PriceApiError::CannotParseData(
                "data is not object".to_string(),
            ))?
            .keys()
            .filter_map(|k| k.parse::<Currency>().ok())
            .collect();
        Ok(ListCurrenciesResult { currencies })
    }
}

// impl<'de> Deserialize<'de> for PriceSource {
//     fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
//     where
//         D: Deserializer<'de>,
//     {
//         deser_fromstr(deserializer)
//     }
// }

// impl Serialize for PriceSource {
//     fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
//     where
//         S: Serializer,
//     {
//         serializer.collect_str(&self)
//     }
// }
