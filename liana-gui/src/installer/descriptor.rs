use async_hwi::{DeviceKind, Version};

use crate::hw::is_compatible_with_tapminiscript;

/// The source of a descriptor public key.
#[derive(Debug, Clone)]
pub enum KeySource {
    Device(DeviceKind, Option<Version>),
    HotSigner,
    Manual,
    Token(String, Provider),
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

    pub fn to_kind(&self) -> KeySourceKind {
        match self {
            Self::Device(_, _) => KeySourceKind::Device,
            Self::HotSigner => KeySourceKind::HotSigner,
            Self::Manual => KeySourceKind::Manual,
            Self::Token(_, _) => KeySourceKind::Token,
        }
    }
}

/// The kind of form required to specify the key.
#[derive(Debug, Clone)]
pub enum KeySourceKind {
    Device,
    HotSigner,
    /// A manually inserted xpub.
    Manual,
    /// A token.
    Token,
}

#[derive(Debug, Clone)]
pub struct Provider {
    name: String,
}
