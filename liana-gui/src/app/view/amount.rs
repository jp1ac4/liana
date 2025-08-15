use liana::miniscript::bitcoin::Amount;
use liana_ui::component::amount::{ToFormattedString, WalletAmount};

use crate::services::fiat::Currency;

pub struct FiatAmount {
    pub amount: f64,
    pub currency: Currency,
}

impl FiatAmount {
    fn _from_btc(amount: Amount, currency: Currency, price: u64) -> Self {
        // Assuming a conversion rate of 1 BTC = 100,000,000 satoshis
        let fiat_amt = amount.to_btc() * price as f64;
        FiatAmount {
            amount: fiat_amt,
            currency,
        }
    }
}

impl WalletAmount for FiatAmount {
    fn to_f64(&self) -> f64 {
        self.amount
    }

    fn sep(&self) -> char {
        ','
    }

    fn num_decimals(&self) -> usize {
        2
    }

    fn sep_decimals(&self) -> bool {
        false
    }

    fn unit(&self) -> String {
        self.currency.to_string()
    }
}

fn _sdsd() {
    let a = FiatAmount::_from_btc(Amount::from_sat(123456789), Currency::AED, 100);
    a.as_formatted_string();
}

// pub trait ToFormattedString {

//     /// Converts the amount to a string representation.
//     fn to_formatted_string(&self) -> String;

//     fn unit(&self) -> String;
// }

// / Formats an f64 as a string with a custom separator and number of decimals,
// / padding the decimal part with zeros if needed.
// / If `sep_fraction` is true, also applies the separator to the decimal part,
// / grouping from the right (e.g., "12345678" -> "12 345 678").
// fn format_f64_with_sep(
//     value: f64,
//     sep: &str,
//     num_decimals: usize,
//     sep_fraction: bool,
// ) -> String {
//     // Format with the requested number of decimals
//     let formatted = format!("{:.*}", num_decimals, value);

//     // Split into integer and fractional parts
//     let (integer, fraction) = match formatted.split_once('.') {
//         Some((i, f)) => (i, f),
//         None => (formatted.as_str(), ""),
//     };

//     // Use format_amount_number_part for integer part (grouping from the right)
//     let int_with_sep = format_amount_number_part(integer, sep);
//     // let fraction_with_sep = format_amount_number_part(fraction, sep);

//     // Pad the fraction with zeros to the right length
//     let padded_fraction = format!("{:0<width$}", fraction, width = num_decimals);

//     // Use format_amount_number_part for fraction if sep_fraction is true
//     let fraction_formatted = if sep_fraction && num_decimals > 0 {
//         format_amount_number_part(&padded_fraction, sep)
//     } else {
//         fraction.to_string()
//     };

//     if num_decimals > 0 {
//         format!("{}.{}", int_with_sep, &fraction_formatted)
//     } else {
//         int_with_sep
//     }
// }

// Format a "part" of a number string with spaces to fit display requirements.
// Currently using French formatting rules so digits are space-separated in groups
// of three, starting from the right side. Incidentally, this works for both the
// integer portion of the number as well as the fraction part.
// Ex:
//   1000 => 1 000
//   100000 => 100 000
// fn format_amount_number_part(s: &str, sep: &str) -> String {
//     let mut part = s
//         .chars()
//         .collect::<Vec<_>>()
//         .rchunks(3)
//         .map(|c| c.iter().collect::<String>())
//         .collect::<Vec<_>>();
//     part.reverse();

//     part.join(sep)
// }

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_format_f64_with_sep() {
//         assert_eq!(
//             format_f64_with_sep(1234567.12345678, " ", 8, false),
//             "1 234 567.12345678"
//         );
//         assert_eq!(
//             format_f64_with_sep(1234567.12345678, ",", 2, false),
//             "1,234,567.12"
//         );
//         assert_eq!(
//             format_f64_with_sep(1234567.12345678, ",", 4, false),
//             "1,234,567.1235"
//         );
//         assert_eq!(
//             format_f64_with_sep(0.000132, " ", 8, false),
//             "0.00013200"
//         );
//         assert_eq!(
//             format_f64_with_sep(0.0, " ", 8, false),
//             "0.00000000"
//         );
//         assert_eq!(
//             format_f64_with_sep(1234567.12345678, " ", 8, true),
//             "1 234 567.12 345 678"
//         );
//         assert_eq!(
//             format_f64_with_sep(0.00799800, " ", 8, true),
//             "0.00 799 800"
//         );
//         assert_eq!(
//             format_f64_with_sep(1000.00799800, " ", 8, true),
//             "1 000.00 799 800"
//         );
//         assert_eq!(
//             format_f64_with_sep(1000.0, " ", 8, true),
//             "1 000.00 000 000"
//         );
//         assert_eq!(
//             format_f64_with_sep(0.00012340, " ", 8, true),
//             "0.00 012 340"
//         );
//         assert_eq!(
//             format_f64_with_sep(0.000132, " ", 8, true),
//             "0.00 013 200"
//         );
//         assert_eq!(
//             format_f64_with_sep(0.0, " ", 8, true),
//             "0.00 000 000"
//         );
//         assert_eq!(
//             format_f64_with_sep(1234.5, ",", 4, true),
//             "1,234.5,000"
//         );
//         assert_eq!(
//             format_f64_with_sep(1234567.0, " ", 0, false),
//             "1 234 567"
//         );
//         assert_eq!(
//             format_f64_with_sep(1234567.0, ",", 0, false),
//             "1,234,567"
//         );
//         assert_eq!(
//             format_f64_with_sep(0.0, " ", 0, false),
//             "0"
//         );
//         assert_eq!(
//             format_f64_with_sep(0.0, ",", 0, false),
//             "0"
//         );
//     }
// }
