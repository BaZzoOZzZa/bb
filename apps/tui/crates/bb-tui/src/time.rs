use chrono::{DateTime, Local};

const MINUTE_MS: i64 = 60 * 1000;
const HOUR_MS: i64 = 60 * MINUTE_MS;
const DAY_MS: i64 = 24 * HOUR_MS;
const WEEK_MS: i64 = 7 * DAY_MS;

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn compact_relative_time(timestamp: i64, now: i64) -> String {
    let diff = now - timestamp;
    if diff < MINUTE_MS {
        return "now".into();
    }
    if diff < HOUR_MS {
        return format!("{}m", diff / MINUTE_MS);
    }
    if diff < DAY_MS {
        return format!("{}h", diff / HOUR_MS);
    }
    if diff < WEEK_MS {
        return format!("{}d", diff / DAY_MS);
    }
    if diff < 5 * WEEK_MS {
        return format!("{}w", diff / WEEK_MS);
    }
    DateTime::from_timestamp_millis(timestamp)
        .map(|date| date.with_timezone(&Local).format("%b %-d").to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_buckets() {
        let now = 100 * WEEK_MS;
        assert_eq!(compact_relative_time(now - 59_000, now), "now");
        assert_eq!(compact_relative_time(now + 5_000, now), "now");
        assert_eq!(compact_relative_time(now - 5 * MINUTE_MS, now), "5m");
        assert_eq!(compact_relative_time(now - 23 * HOUR_MS, now), "23h");
        assert_eq!(compact_relative_time(now - 6 * DAY_MS, now), "6d");
        assert_eq!(compact_relative_time(now - 4 * WEEK_MS, now), "4w");
        let old = compact_relative_time(now - 6 * WEEK_MS, now);
        assert!(old.split(' ').count() == 2, "{old}");
    }
}
