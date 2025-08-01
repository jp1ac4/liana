pub mod api;
pub mod currency;
pub mod source;

use async_trait::async_trait;

use api::{GetPriceResult, PriceApi};
pub use currency::Currency;
pub use source::PriceSource;

use crate::{
    fiat::api::{ListCurrenciesResult, PriceApiError},
    http::ResponseExt,
};

pub struct PriceClient<C> {
    inner: C,
    source: PriceSource,
}

impl<C> PriceClient<C> {
    pub fn new(inner: C, source: PriceSource) -> Self {
        Self { inner, source }
    }
}

impl<C: Default> PriceClient<C> {
    pub fn default_from_source(source: PriceSource) -> Self {
        Self::new(C::default(), source)
    }
}

async fn get_data(client: &reqwest::Client, url: &str) -> Result<serde_json::Value, PriceApiError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| PriceApiError::RequestFailed(e.to_string()))?
        .check_success()
        .await
        .map_err(PriceApiError::NotSuccessResponse)?;
    let data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| PriceApiError::CannotParseResponse(e.to_string()))?;
    Ok(data)
}

#[async_trait]
impl PriceApi for PriceClient<reqwest::Client> {
    async fn get_price(&self, currency: Currency) -> Result<GetPriceResult, PriceApiError> {
        let url = match self.source {
            PriceSource::MempoolSpace => "https://mempool.space/api/v1/prices".to_string(),
            PriceSource::CoinGecko => format!("https://api.coingecko.com/api/v3/simple/price?vs_currencies={}&include_last_updated_at=true", currency),
        };
        let data = get_data(&self.inner, &url).await?;
        let (value, timestamp) = match self.source {
            PriceSource::MempoolSpace => {
                let value = data
                    .get(currency.to_string())
                    .ok_or(PriceApiError::CurrencyNotFound)?
                    .as_u64()
                    .ok_or(PriceApiError::CannotParsePrice)?;
                let timestamp = data.get("timestamp").and_then(|t| t.as_u64());
                (value, timestamp)
            }
            PriceSource::CoinGecko => {
                let value = data
                    .get(currency.to_string())
                    .ok_or(PriceApiError::CurrencyNotFound)?
                    .as_u64()
                    .ok_or(PriceApiError::CannotParsePrice)?;
                let timestamp = data.get("timestamp").and_then(|t| t.as_u64());
                (value, timestamp)
            }
        };
        Ok(GetPriceResult { value, timestamp })
    }

    async fn list_currencies(&self) -> Result<ListCurrenciesResult, PriceApiError> {
        let url = match self.source {
            PriceSource::MempoolSpace => "https://mempool.space/api/v1/prices".to_string(),
            PriceSource::CoinGecko => {
                "https://api.coingecko.com/api/v3/simple/supported_vs_currencies".to_string()
            }
        };
        let currencies: Vec<_> = get_data(&self.inner, &url)
            .await?
            .as_object()
            .ok_or(PriceApiError::CannotParsePrice)?
            .keys()
            .filter_map(|k| k.parse().ok())
            .collect();

        Ok(ListCurrenciesResult { currencies })
    }
}

// pub async fn do_stuff() {
//     let price_client = PriceClient::default_from_source(PriceSource::CoinGecko);
//     let p = price_client.get_price(Currency::CAD).await;
// }
