use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::utils::serde::deser_fromstr;

macro_rules! currency_enum {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq)]
        pub enum $name {
            #[default]
            $($variant,)*
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self {
                    $(Self::$variant => write!(f, stringify!($variant)),)*
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s.to_uppercase().as_str() {
                    $(stringify!($variant) => Ok(Self::$variant),)*
                    _ => Err("Invalid currency".to_string()),
                }
            }
        }
    };
}

currency_enum!(Currency {
    USD,
    AED,
    ARS,
    AUD,
    BDT,
    BHT,
    BMD,
    BRL,
    CAD,
    CHF,
    CLP,
    CNY,
    CZK,
    DKK,
    EUR,
    GBP,
    GEL,
    HKD,
    HUF,
    IDR,
    ILS,
    INR,
    JPY,
    KRW,
    KWD,
});

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deser_fromstr(deserializer)
    }
}

impl Serialize for Currency {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self)
    }
}
