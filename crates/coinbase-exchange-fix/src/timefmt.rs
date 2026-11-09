use chrono::{DateTime, NaiveDateTime, Utc};

use crate::error::{Error, Result};

/// `YYYYMMDD-HH:MM:SS.sss`, the only SendingTime shape Coinbase will sign.
pub fn format_millis(t: DateTime<Utc>) -> String {
    t.format("%Y%m%d-%H:%M:%S%.3f").to_string()
}

pub fn is_fix_millis(s: &str) -> bool {
    if s.len() != 21 {
        return false;
    }
    let b = s.as_bytes();
    b[8] == b'-'
        && b[11] == b':'
        && b[14] == b':'
        && b[17] == b'.'
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 11 | 14 | 17 => true,
            _ => c.is_ascii_digit(),
        })
}

/// Parse a FIX timestamp. Fractional seconds may be millis, micros, or nanos.
pub fn parse_timestamp(s: &str) -> Result<DateTime<Utc>> {
    let naive = NaiveDateTime::parse_from_str(s, "%Y%m%d-%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y%m%d-%H:%M:%S"))
        .map_err(|_| Error::codec(format!("bad timestamp `{s}`")))?;
    Ok(naive.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn millis_round_trip() {
        let t = Utc.with_ymd_and_hms(2023, 8, 22, 20, 43, 30).unwrap()
            + chrono::Duration::milliseconds(12);
        let s = format_millis(t);
        assert_eq!(s, "20230822-20:43:30.012");
        assert!(is_fix_millis(&s));
        assert!(!is_fix_millis("20230822-20:43:30"));
        assert!(!is_fix_millis("20230822-20:43:30.123456"));
        assert_eq!(parse_timestamp(&s).unwrap(), t);
        assert!(parse_timestamp("20230822-20:43:30.123456").is_ok());
    }
}
