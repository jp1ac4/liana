pub use bitcoin::Amount;
use iced::Color;

use crate::{color, component::text::*, widget::*};

pub trait WalletAmount {
    fn to_f64(&self) -> f64;

    fn sep(&self) -> char;

    fn num_decimals(&self) -> usize;

    fn sep_decimals(&self) -> bool;

    fn unit(&self) -> String;
}

pub trait ToFormattedString {
    /// Converts the amount to a string representation with formatting.
    fn as_formatted_string(&self) -> String;
}

impl<A: WalletAmount> ToFormattedString for A {
    /// Converts the amount to a string representation with formatting.
    fn as_formatted_string(&self) -> String {
        format_f64_with_sep(
            self.to_f64(),
            &self.sep().to_string(),
            self.num_decimals(),
            self.sep_decimals(),
        )
    }
}

impl WalletAmount for Amount {
    fn to_f64(&self) -> f64 {
        self.to_btc()
    }

    fn sep(&self) -> char {
        ' '
    }

    fn num_decimals(&self) -> usize {
        8
    }

    fn sep_decimals(&self) -> bool {
        true
    }

    fn unit(&self) -> String {
        "BTC".to_string()
    }
}

/// Formats an f64 as a string with a custom separator and number of decimals,
/// padding the decimal part with zeros if needed.
/// If `sep_fraction` is true, also applies the separator to the decimal part,
/// grouping from the right (e.g., "12345678" -> "12 345 678").
pub fn format_f64_with_sep(
    value: f64,
    sep: &str,
    num_decimals: usize,
    sep_fraction: bool,
) -> String {
    // Format with the requested number of decimals
    let formatted = format!("{:.*}", num_decimals, value);

    // Split into integer and fractional parts
    let (integer, fraction) = match formatted.split_once('.') {
        Some((i, f)) => (i, f),
        None => (formatted.as_str(), ""),
    };

    // Use format_amount_number_part for integer part (grouping from the right)
    let int_with_sep = format_amount_number_part(integer, sep);
    // let fraction_with_sep = format_amount_number_part(fraction, sep);

    // Pad the fraction with zeros to the right length
    let padded_fraction = format!("{:0<width$}", fraction, width = num_decimals);

    // Use format_amount_number_part for fraction if sep_fraction is true
    let fraction_formatted = if sep_fraction && num_decimals > 0 {
        format_amount_number_part(&padded_fraction, sep)
    } else {
        fraction.to_string()
    };

    if num_decimals > 0 {
        format!("{}.{}", int_with_sep, &fraction_formatted)
    } else {
        int_with_sep
    }
}

/// Amount with default size and colors.
pub fn amount<'a, A: WalletAmount, T: 'a>(a: &A) -> Row<'a, T> {
    amount_with_size(a, P1_SIZE)
}

/// Amount with default colors.
pub fn amount_with_size<'a, A: WalletAmount, T: 'a>(a: &A, size: u16) -> Row<'a, T> {
    amount_with_size_and_colors(a, size, color::GREY_3, None)
}

/// Amount with the given size and colors.
///
/// `color_before` is the color to use before the first non-zero
/// value in `a`.
///
/// `color_after` is the color to use from the first non-zero
/// value in `a` onwards. If `None`, the default theme value
/// will be used.
pub fn amount_with_size_and_colors<'a, A: WalletAmount, T: 'a>(
    a: &A,
    size: u16,
    color_before: Color,
    color_after: Option<Color>,
) -> Row<'a, T> {
    render_amount(a, size, color_before, color_after)
}

pub fn unconfirmed_amount_with_size<'a, A: WalletAmount, T: 'a>(a: &A, size: u16) -> Row<'a, T> {
    render_unconfirmed_amount(a, size)
}

//
// Helpers
//

// Format a "part" of a number string with spaces to fit display requirements.
// Currently using French formatting rules so digits are space-separated in groups
// of three, starting from the right side. Incidentally, this works for both the
// integer portion of the number as well as the fraction part.
// Ex:
//   1000 => 1 000
//   100000 => 100 000
fn format_amount_number_part(s: &str, sep: &str) -> String {
    let mut part = s
        .chars()
        .collect::<Vec<_>>()
        .rchunks(3)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>();
    part.reverse();

    part.join(sep)
}

// Helper functions split a string at the first occurence of a non-zero integer (where
// the amount starts).
fn split_at_first_non_zero(s: &str) -> Option<(String, String)> {
    for (index, c) in s.char_indices() {
        if c.is_ascii_digit() && c != '0' {
            let (before, after) = s.split_at(index);
            return Some((before.to_string(), after.to_string()));
        }
    }
    None
}

// Build the rendering elements for displaying a Bitcoin amount.
// The text should be bolded beginning where the BTC amount is non-zero.
fn render_amount<'a, A: WalletAmount, T: 'a>(
    amount: &A,
    size: u16,
    color_before: Color,
    color_after: Option<Color>,
) -> Row<'a, T> {
    let spacing = if size > P1_SIZE { 10 } else { 5 };
    let amt_str = amount.as_formatted_string();

    let (before, after) = match split_at_first_non_zero(&amt_str) {
        Some((b, a)) => (b, a),
        None => (amt_str, String::from("")),
    };

    let mut child_after = text(after).size(size).bold();
    if let Some(color_after) = color_after {
        child_after = child_after.color(color_after);
    }
    let row = Row::new()
        .push(text(before).size(size).color(color_before))
        .push(child_after);

    Row::with_children(vec![
        row.into(),
        text(amount.unit()).size(size).color(color_before).into(),
    ])
    .spacing(spacing)
    .align_y(iced::Alignment::Center)
}

// Build the rendering elements for displaying a Bitcoin amount.
fn render_unconfirmed_amount<'a, A: WalletAmount, T: 'a>(amount: &A, size: u16) -> Row<'a, T> {
    let spacing = if size > P1_SIZE { 10 } else { 5 };

    Row::with_children(vec![
        text(amount.as_formatted_string())
            .size(size)
            .color(color::GREY_3)
            .into(),
        text(amount.unit()).size(size).color(color::GREY_3).into(),
    ])
    .spacing(spacing)
    .align_y(iced::Alignment::Center)
}

#[cfg(test)]
mod tests {
    use super::*;

    // #[test]
    // fn test_amount_as_str() {
    //     assert_eq!(
    //         "0.00 799 800",
    //         WalletAmount::Btc(bitcoin::Amount::from_btc(0.00799800).unwrap()).as_string()
    //     );
    //     assert_eq!(
    //         "1 000.00 799 800",
    //         WalletAmount::Btc(bitcoin::Amount::from_btc(1000.00799800).unwrap()).as_string()
    //     );
    //     assert_eq!(
    //         "1 000.00 000 000",
    //         WalletAmount::Btc(bitcoin::Amount::from_btc(1000.0).unwrap()).as_string()
    //     );
    //     assert_eq!(
    //         "0.00 012 340",
    //         WalletAmount::Btc(bitcoin::Amount::from_btc(0.00012340).unwrap()).as_string()
    //     )
    // }

    #[test]
    fn test_format_f64_with_sep() {
        assert_eq!(
            format_f64_with_sep(1234567.12345678, " ", 8, false),
            "1 234 567.12345678"
        );
        assert_eq!(
            format_f64_with_sep(1234567.12345678, ",", 2, false),
            "1,234,567.12"
        );
        assert_eq!(
            format_f64_with_sep(1234567.12345678, ",", 4, false),
            "1,234,567.1235"
        );
        assert_eq!(format_f64_with_sep(0.000132, " ", 8, false), "0.00013200");
        assert_eq!(format_f64_with_sep(0.0, " ", 8, false), "0.00000000");
        assert_eq!(
            format_f64_with_sep(1234567.12345678, " ", 8, true),
            "1 234 567.12 345 678"
        );
        assert_eq!(
            format_f64_with_sep(0.00799800, " ", 8, true),
            "0.00 799 800"
        );
        assert_eq!(
            format_f64_with_sep(1000.00799800, " ", 8, true),
            "1 000.00 799 800"
        );
        assert_eq!(
            format_f64_with_sep(1000.0, " ", 8, true),
            "1 000.00 000 000"
        );
        assert_eq!(
            format_f64_with_sep(0.00012340, " ", 8, true),
            "0.00 012 340"
        );
        assert_eq!(format_f64_with_sep(0.000132, " ", 8, true), "0.00 013 200");
        assert_eq!(format_f64_with_sep(0.0, " ", 8, true), "0.00 000 000");
        assert_eq!(format_f64_with_sep(1234.5, ",", 4, true), "1,234.5,000");
        assert_eq!(format_f64_with_sep(1234567.0, " ", 0, false), "1 234 567");
        assert_eq!(format_f64_with_sep(1234567.0, ",", 0, false), "1,234,567");
        assert_eq!(format_f64_with_sep(0.0, " ", 0, false), "0");
        assert_eq!(format_f64_with_sep(0.0, ",", 0, false), "0");
    }
}
