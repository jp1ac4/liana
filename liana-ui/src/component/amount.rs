pub use bitcoin::Amount;
use iced::Color;

use crate::{color, component::text::*, widget::*};

pub struct AmountFormatter<'a> {
    /// Thousands separator.
    pub sep: &'a str,
    /// Number of decimal places to show.
    ///
    /// If `num_decimals` is 0, no decimal part will be shown.
    ///
    /// If `num_decimals` is greater than 0,
    /// the decimal part will be padded with zeros
    /// to the right to match the specified number of decimals.
    pub num_decimals: usize,
    /// Whether to separate the decimal part with `sep`.
    ///
    /// If `true`, the decimal part will be grouped from the right.
    ///
    /// If `false`, the decimal part will not be grouped.
    ///
    /// If `num_decimals` is 0, this has no effect.
    pub sep_decimals: bool,
}

impl<'a> AmountFormatter<'a> {
    pub fn format(&self, value: f64) -> String {
        format_f64_with_sep(value, self.sep, self.num_decimals, self.sep_decimals)
    }
}

pub trait WalletAmount {
    fn as_formatted_string(&self) -> String;
    fn unit(&self) -> String;
}

impl WalletAmount for Amount {
    fn as_formatted_string(&self) -> String {
        // Use your AmountFormatter or formatting logic directly
        let formatter = AmountFormatter {
            sep: " ",
            num_decimals: 8,
            sep_decimals: true,
        };
        formatter.format(self.to_btc())
    }

    fn unit(&self) -> String {
        "BTC".to_string()
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

/// Formats an f64 as a string with a custom separator and number of decimals,
/// padding the decimal part with zeros if needed.
/// If `sep_decimals` is true, also applies the separator to the decimal part,
/// grouping from the right (e.g., "12345678" -> "12 345 678").
pub fn format_f64_with_sep(
    value: f64,
    sep: &str,
    num_decimals: usize,
    sep_decimals: bool,
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

    // Pad the fraction with zeros to the right length
    let mut padded_fraction = String::with_capacity(num_decimals);
    padded_fraction.push_str(fraction);
    while padded_fraction.len() < num_decimals {
        padded_fraction.push('0');
    }

    // Use format_amount_number_part for decimals if sep_decimals is true
    let fraction_formatted = if sep_decimals && num_decimals > 0 {
        format_amount_number_part(&padded_fraction, sep)
    } else {
        padded_fraction
    };

    if num_decimals > 0 {
        format!("{}.{}", int_with_sep, &fraction_formatted)
    } else {
        int_with_sep
    }
}

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

    #[test]
    fn test_amount_as_str() {
        assert_eq!(
            "0.00 799 800",
            bitcoin::Amount::from_btc(0.00799800)
                .unwrap()
                .as_formatted_string()
        );
        assert_eq!(
            "1 000.00 799 800",
            bitcoin::Amount::from_btc(1000.00799800)
                .unwrap()
                .as_formatted_string()
        );
        assert_eq!(
            "1 000.00 000 000",
            bitcoin::Amount::from_btc(1000.0)
                .unwrap()
                .as_formatted_string()
        );
        assert_eq!(
            "0.00 012 340",
            bitcoin::Amount::from_btc(0.00012340)
                .unwrap()
                .as_formatted_string()
        )
    }

    #[test]
    fn test_format_f64_with_sep() {
        assert_eq!(
            format_f64_with_sep(1234567.12345678, " ", 8, false),
            "1 234 567.12345678"
        );
        assert_eq!(
            format_f64_with_sep(1234567.12345678, " ", 8, true),
            "1 234 567.12 345 678"
        );

        assert_eq!(
            format_f64_with_sep(1234567.12345678, ",", 2, false),
            "1,234,567.12"
        );
        assert_eq!(
            format_f64_with_sep(1234567.12345678, ",", 2, true),
            "1,234,567.12"
        );

        assert_eq!(
            format_f64_with_sep(1234567.12345678, ",", 4, false),
            "1,234,567.1235"
        );
        assert_eq!(
            format_f64_with_sep(1234567.12345678, ",", 4, true),
            "1,234,567.1,235"
        );

        assert_eq!(format_f64_with_sep(0.000132, " ", 8, false), "0.00013200");
        assert_eq!(format_f64_with_sep(0.000132, " ", 8, true), "0.00 013 200");

        assert_eq!(format_f64_with_sep(0.0, " ", 8, false), "0.00000000");
        assert_eq!(format_f64_with_sep(0.0, " ", 8, true), "0.00 000 000");

        assert_eq!(
            format_f64_with_sep(1000.00799800, " ", 8, false),
            "1 000.00799800"
        );
        assert_eq!(
            format_f64_with_sep(1000.00799800, " ", 8, true),
            "1 000.00 799 800"
        );

        assert_eq!(format_f64_with_sep(1000.0, " ", 8, false), "1 000.00000000");
        assert_eq!(
            format_f64_with_sep(1000.0, " ", 8, true),
            "1 000.00 000 000"
        );

        assert_eq!(format_f64_with_sep(1234567.0, " ", 0, false), "1 234 567");
        assert_eq!(format_f64_with_sep(1234567.0, " ", 0, true), "1 234 567");

        assert_eq!(format_f64_with_sep(1234567.0, ",", 0, false), "1,234,567");
        assert_eq!(format_f64_with_sep(1234567.0, ",", 0, true), "1,234,567");

        assert_eq!(format_f64_with_sep(0.0, " ", 0, false), "0");
        assert_eq!(format_f64_with_sep(0.0, " ", 0, true), "0");

        assert_eq!(format_f64_with_sep(0.0, ",", 0, false), "0");
        assert_eq!(format_f64_with_sep(0.0, ",", 0, true), "0");
    }
}
