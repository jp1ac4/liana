use serde::{Deserialize, Serialize};

use crate::fiat::{Currency, PriceSource};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct PriceSetting {
    pub currency: Currency,
    pub source: PriceSource,
    pub is_enabled: bool,
}
