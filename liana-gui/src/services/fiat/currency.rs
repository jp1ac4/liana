macro_rules! currency_enum {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, Default)]
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
    USD, // macro sets first variant as the default
    AED,
    ARS,
    AUD,
    BDT,
    BHD,
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
    LKR,
    MMK,
    MXN,
    MYR,
    NGN,
    NOK,
    NZD,
    PHP,
    PKR,
    PLN,
    RUB,
    SAR,
    SEK,
    SGD,
    THB,
    TRY,
    TWD,
    UAH,
    VEF,
    VND,
    ZAR,
});
