use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, Timelike};
use wafu_core::{LocalDateTime, Timestamp};

pub(crate) fn timestamp(at: SystemTime) -> Timestamp {
    let local: DateTime<Local> = at.into();
    Timestamp {
        at,
        local: LocalDateTime {
            year: local.year(),
            month: narrow(local.month()),
            day: narrow(local.day()),
            hour: narrow(local.hour()),
            minute: narrow(local.minute()),
            second: narrow(local.second()),
        },
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

        let stamp = timestamp(at);

        assert_eq!(stamp.at, at);
        assert_eq!(stamp.local.year, 2023);
        assert!((1..=12).contains(&stamp.local.month));
        assert!((1..=31).contains(&stamp.local.day));
        assert!(stamp.local.hour < 24);
        assert!(stamp.local.minute < 60);
        assert!(stamp.local.second < 60);
    }
}
