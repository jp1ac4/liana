use std::str::FromStr;

use async_hwi::{DeviceKind, Version};

use liana::miniscript::{bitcoin::bip32::Fingerprint, descriptor::DescriptorPublicKey};

use crate::hw::is_compatible_with_tapminiscript;

/// The source of a descriptor public key.
#[derive(Debug, Clone)]
pub enum KeySource {
    Device(DeviceKind, Option<Version>),
    HotSigner,
    Manual,
    Token(TokenResource),
}

impl KeySource {
    pub fn device_kind(&self) -> Option<&DeviceKind> {
        if let KeySource::Device(ref device_kind, _) = self {
            Some(device_kind)
        } else {
            None
        }
    }

    pub fn device_version(&self) -> Option<&Version> {
        if let KeySource::Device(_, ref version) = self {
            version.as_ref()
        } else {
            None
        }
    }

    pub fn is_compatible_taproot(&self) -> bool {
        if let KeySource::Device(ref device_kind, ref version) = self {
            is_compatible_with_tapminiscript(device_kind, version.as_ref())
        } else {
            true
        }
    }

    pub fn kind(&self) -> KeySourceKind {
        match self {
            Self::Device(_, _) => KeySourceKind::Device,
            Self::HotSigner => KeySourceKind::HotSigner,
            Self::Manual => KeySourceKind::Manual,
            Self::Token(_) => KeySourceKind::Token,
        }
    }
}

/// The kind of form required to specify the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySourceKind {
    Device,
    HotSigner,
    /// A manually inserted xpub.
    Manual,
    /// A token.
    Token,
}

#[derive(Debug, Clone)]
pub struct TokenResource {
    pub token: String,
    pub fingerprint: Fingerprint,
    pub key: DescriptorPublicKey,
    pub provider_name: String,
}

// FIXME: fix error type, add API call
pub async fn get_token_resource(token: String) -> Result<TokenResource, std::io::Error> {
    let key = DescriptorPublicKey::from_str("[8c3ffb6e/48'/1'/0'/2']tpubDEMt3bpQMa99W81K9h8f2FJH1C81eSd6bbSkBP8tcqQHAfSKvuGp2fz6xiVpfShzT9sKPx7DVBphChjxvNd15WcbsCca5oVz1AcUTWHxkdS/<0';1>/*").unwrap();
    Ok(TokenResource {
        token,
        fingerprint: key.master_fingerprint(),
        key,
        provider_name: "Keys R Us".to_string(),
    })
}
