use std::{convert::TryFrom, str::FromStr};

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
            Self::Token(TokenResource { kind, .. }) => KeySourceKind::Token(*kind),
        }
    }

    // TODO: change to something that gives more info about token
    pub fn token(&self) -> Option<String> {
        if let KeySource::Token(tr) = self {
            Some(tr.token.clone())
        } else {
            None
        }
    }
}

/// The kind of token.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Copy)]
#[repr(u8)]
pub enum TokenKind {
    SafetyNet = 1,
    Cosigner = 2,
}

impl TryFrom<u8> for TokenKind {
    type Error = ();

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            t if t == TokenKind::SafetyNet as u8 => Ok(TokenKind::SafetyNet),
            t if t == TokenKind::Cosigner as u8 => Ok(TokenKind::Cosigner),
            _ => Err(()),
        }
    }
}

/// The kind of form required to specify the key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeySourceKind {
    Device,
    HotSigner,
    /// A manually inserted xpub.
    Manual,
    /// A token.
    Token(TokenKind),
}

#[derive(Debug, Clone)]
pub struct TokenResource {
    pub token: String,
    pub kind: TokenKind,
    pub fingerprint: Fingerprint,
    pub key: DescriptorPublicKey,
    pub provider_name: String,
}

// FIXME: fix error type, add API call
pub async fn get_token_resource(token: String) -> Result<TokenResource, std::io::Error> {
    let key = DescriptorPublicKey::from_str("[8c3ffb6e/48'/1'/0'/2']tpubDEMt3bpQMa99W81K9h8f2FJH1C81eSd6bbSkBP8tcqQHAfSKvuGp2fz6xiVpfShzT9sKPx7DVBphChjxvNd15WcbsCca5oVz1AcUTWHxkdS").unwrap();
    Ok(TokenResource {
        token,
        kind: TokenKind::SafetyNet,
        fingerprint: key.master_fingerprint(),
        key,
        provider_name: "Keys R Us".to_string(),
    })
}

#[derive(Debug, Clone, Copy)]
pub enum PathSequence {
    Primary,
    Recovery(u16),
    SafetyNet,
}

impl PathSequence {
    pub fn as_u16(&self) -> u16 {
        match self {
            Self::Primary => 0,
            Self::Recovery(s) => *s,
            Self::SafetyNet => u16::MAX,
        }
    }

    pub fn path_kind(&self) -> PathKind {
        match self {
            Self::Primary => PathKind::Primary,
            Self::Recovery(_) => PathKind::Recovery,
            Self::SafetyNet => PathKind::SafetyNet,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PathKind {
    Primary,
    Recovery,
    SafetyNet,
}

impl PathKind {
    pub fn can_choose_key_source_kind(&self, source_kind: &KeySourceKind) -> bool {
        match (self, source_kind) {
            // Safety net path only allows safety net tokens.
            (Self::SafetyNet, KeySourceKind::Token(TokenKind::SafetyNet)) => true,
            (Self::SafetyNet, _) => false,
            // Safety net tokens cannot be used in any other path kind.
            (_, KeySourceKind::Token(TokenKind::SafetyNet)) => false,
            _ => true,
        }
    }

    // pub fn can_edit_sequence(&self) -> bool {
    //     match self {
    //         Self::Recovery(_) => true,
    //         _ => false,
    //     }
    // }
}
