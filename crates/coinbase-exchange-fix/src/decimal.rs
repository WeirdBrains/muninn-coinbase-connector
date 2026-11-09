use crate::error::{Error, Result};
use rust_decimal::Decimal;
use std::str::FromStr;

pub(crate) fn decimal(s: &str) -> Result<Decimal> {
    if !is_plain_decimal(s) {
        return Err(Error::rejected(format!("`{s}` is not a plain decimal")));
    }
    Decimal::from_str(s).map_err(|e| Error::rejected(format!("`{s}` is not a decimal: {e}")))
}

pub(crate) fn positive(s: &str, what: &str) -> Result<Decimal> {
    let d = decimal(s)?;
    if d.is_sign_negative() || d.is_zero() {
        return Err(Error::rejected(format!("{what} must be positive")));
    }
    Ok(d)
}

/// Significant digits after stripping trailing fractional zeros.
pub(crate) fn significant_digits(s: &str) -> Result<usize> {
    let d = decimal(s)?;
    if d.is_zero() {
        return Ok(1);
    }
    Ok(d.normalize().mantissa().unsigned_abs().to_string().len())
}

pub(crate) fn is_multiple(qty: &str, increment: &str) -> Result<bool> {
    let q = positive(qty, "order qty")?;
    let inc = positive(increment, "base increment")?;
    Ok((q % inc).is_zero() && q >= inc)
}

fn is_plain_decimal(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_digit() {
        return false;
    }
    let mut dot = false;
    let mut prev_digit = true;
    for c in chars {
        match c {
            '0'..='9' => prev_digit = true,
            '.' if !dot => {
                dot = true;
                prev_digit = false;
            }
            _ => return false,
        }
    }
    prev_digit
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_exponents_and_signs() {
        assert!(decimal("1e-2").is_err());
        assert!(decimal("-1").is_err());
        assert!(decimal("1.").is_err());
        assert!(decimal(".5").is_err());
        assert!(positive("0.5", "price").is_ok());
    }

    #[test]
    fn counts_significant_digits() {
        assert_eq!(significant_digits("1000").unwrap(), 4);
        assert_eq!(significant_digits("1.2300").unwrap(), 3);
        assert!(significant_digits("1.234567890123456").unwrap() > 15);
        assert!(significant_digits("1.23456789012345").unwrap() <= 15);
    }

    #[test]
    fn multiples() {
        assert!(is_multiple("0.00000002", "0.00000001").unwrap());
        assert!(!is_multiple("0.000000015", "0.00000001").unwrap());
        assert!(!is_multiple("0.000000005", "0.00000001").unwrap());
    }
}
