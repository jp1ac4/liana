pub mod auth;
pub mod backend;

use liana::miniscript::bitcoin;

use serde::Deserialize;

const BACKEND_URL: &str = "http://localhost:8080";

#[derive(Debug, Clone, Deserialize)]
pub struct ServiceConfig {
    pub backend_api_url: String,
}

pub async fn get_service_config() -> Result<ServiceConfig, reqwest::Error> {
    Ok(ServiceConfig {
        backend_api_url: BACKEND_URL.to_string(),
    })
}
