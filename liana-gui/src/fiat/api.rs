use super::currency::Currency;

use async_trait::async_trait;

use crate::http::NotSuccessResponseInfo;

#[derive(Debug, Clone)]
pub struct GetPriceResult {
    pub value: u64,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ListCurrenciesResult {
    pub currencies: Vec<Currency>,
}

pub enum PriceApiError {
    CurrencyNotFound,
    CannotParsePrice,
    RequestFailed(String),
    NotSuccessResponse(NotSuccessResponseInfo),
    CannotParseResponse(String),
}

#[async_trait]
pub trait PriceApi {
    async fn get_price(&self, currency: Currency) -> Result<GetPriceResult, PriceApiError>;

    async fn list_currencies(&self) -> Result<ListCurrenciesResult, PriceApiError>;
}
