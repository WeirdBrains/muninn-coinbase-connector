//! When the gateway logs the session out.
//!
//! FIX 5 sessions are logged out every Saturday at 18:00 UTC (1PM ET).
//! Deploys, which also send logout, land Monday, Wednesday, and Thursday
//! at 19:00 UTC, except US federal holidays (not enumerated here).

use chrono::{DateTime, Datelike, Duration, NaiveTime, TimeZone, Utc, Weekday};

/// Next Saturday 18:00 UTC strictly after `now`, or later today if that instant is still ahead.
pub fn next_weekly_logout(now: DateTime<Utc>) -> DateTime<Utc> {
    next_at(
        now,
        &[Weekday::Sat],
        NaiveTime::from_hms_opt(18, 0, 0).unwrap(),
    )
}

/// Next Monday, Wednesday, or Thursday 19:00 UTC strictly after `now`.
pub fn next_deploy_logout(now: DateTime<Utc>) -> DateTime<Utc> {
    next_at(
        now,
        &[Weekday::Mon, Weekday::Wed, Weekday::Thu],
        NaiveTime::from_hms_opt(19, 0, 0).unwrap(),
    )
}

fn next_at(now: DateTime<Utc>, days: &[Weekday], clock: NaiveTime) -> DateTime<Utc> {
    for add in 0..14 {
        let date = now.date_naive() + Duration::days(add);
        if days.contains(&date.weekday()) {
            let at = Utc.from_utc_datetime(&date.and_time(clock));
            if at > now {
                return at;
            }
        }
    }
    now
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn saturday_reset_and_deploy_window() {
        let sunday = utc(2026, 11, 8, 22, 0);
        assert_eq!(next_weekly_logout(sunday), utc(2026, 11, 14, 18, 0));
        assert_eq!(
            next_weekly_logout(utc(2026, 11, 14, 17, 59)),
            utc(2026, 11, 14, 18, 0)
        );
        assert_eq!(
            next_weekly_logout(utc(2026, 11, 14, 18, 0)),
            utc(2026, 11, 21, 18, 0)
        );
        assert_eq!(next_deploy_logout(sunday), utc(2026, 11, 9, 19, 0));
        assert_eq!(
            next_deploy_logout(utc(2026, 11, 9, 19, 0)),
            utc(2026, 11, 11, 19, 0)
        );
    }
}
