use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeDelta, TimeZone, Timelike};
use wafu_core::{LocalDateTime, Timestamp};

pub(crate) fn timestamp(at: SystemTime) -> Option<Timestamp> {
    let utc = utc(at)?;
    let local = utc.checked_add_offset(Local.offset_from_utc_datetime(&utc))?;
    Some(Timestamp {
        at,
        local: LocalDateTime {
            year: local.year(),
            month: narrow(local.month()),
            day: narrow(local.day()),
            hour: narrow(local.hour()),
            minute: narrow(local.minute()),
            second: narrow(local.second()),
        },
    })
}

fn utc(at: SystemTime) -> Option<NaiveDateTime> {
    let epoch = DateTime::UNIX_EPOCH.naive_utc();
    match at.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(after) => epoch.checked_add_signed(TimeDelta::from_std(after).ok()?),
        Err(before) => epoch.checked_sub_signed(TimeDelta::from_std(before.duration()).ok()?),
    }
}

fn narrow(value: u32) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn timestamp_keeps_the_instant_and_fills_a_plausible_date() {
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let stamp = timestamp(at).unwrap();

        assert_eq!(stamp.at, at);
        assert_eq!(stamp.local.year, 2023);
        assert!((1..=12).contains(&stamp.local.month));
        assert!((1..=31).contains(&stamp.local.day));
        assert!(stamp.local.hour < 24);
        assert!(stamp.local.minute < 60);
        assert!(stamp.local.second < 60);
    }

    #[test]
    fn timestamp_fills_a_date_before_the_epoch() {
        let at = SystemTime::UNIX_EPOCH - Duration::from_hours(87_648);

        let stamp = timestamp(at).unwrap();

        assert_eq!(stamp.at, at);
        assert_eq!(stamp.local.year, 1960);
    }

    #[test]
    fn timestamp_gives_none_for_an_unrepresentable_instant() {
        let Some(at) = SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(100_000_000_000_000))
        else {
            return;
        };

        assert_eq!(timestamp(at), None);
    }
}
