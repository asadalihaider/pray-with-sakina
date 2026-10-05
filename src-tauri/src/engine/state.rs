use chrono::{DateTime, Utc};

use super::types::{LoggedStatus, PrayerState};
use super::windows::Window;

/// Build plan section 4.4. State is derived, not stored as its own field:
/// a persisted log always wins, otherwise it falls out of `now` vs the
/// window. This is what makes sleep/wake correct for free — the first call
/// after waking simply finds `now` past `window.end` with nothing logged,
/// which is exactly "Unlogged".
pub fn compute_state(
    now: DateTime<Utc>,
    window: &Window,
    logged: Option<(LoggedStatus, DateTime<Utc>)>,
) -> PrayerState {
    if let Some((status, at)) = logged {
        return match status {
            LoggedStatus::Prayed => PrayerState::Prayed { at },
            LoggedStatus::Missed => PrayerState::Missed,
        };
    }

    if now < window.start {
        PrayerState::Upcoming
    } else if window.contains(now) {
        PrayerState::Active
    } else {
        PrayerState::Unlogged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn window() -> Window {
        let start = Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap();
        Window {
            start,
            end: start + Duration::hours(3),
        }
    }

    #[test]
    fn before_window_is_upcoming() {
        let w = window();
        let now = w.start - Duration::minutes(1);
        assert_eq!(compute_state(now, &w, None), PrayerState::Upcoming);
    }

    #[test]
    fn inside_window_with_no_log_is_active() {
        let w = window();
        let now = w.start + Duration::hours(1);
        assert_eq!(compute_state(now, &w, None), PrayerState::Active);
    }

    #[test]
    fn after_window_with_no_log_is_unlogged() {
        let w = window();
        let now = w.end + Duration::minutes(1);
        assert_eq!(compute_state(now, &w, None), PrayerState::Unlogged);
    }

    #[test]
    fn waking_up_after_the_window_closed_is_unlogged_with_no_extra_handling() {
        // Simulates sleep: the laptop was suspended through the whole
        // window and only checks state again well after `window.end`.
        let w = window();
        let woke_up_at = w.end + Duration::hours(6);
        assert_eq!(compute_state(woke_up_at, &w, None), PrayerState::Unlogged);
    }

    #[test]
    fn logged_prayed_wins_even_after_the_window_closed() {
        let w = window();
        let prayed_at = w.start + Duration::minutes(10);
        let now = w.end + Duration::hours(1);
        assert_eq!(
            compute_state(now, &w, Some((LoggedStatus::Prayed, prayed_at))),
            PrayerState::Prayed { at: prayed_at }
        );
    }

    #[test]
    fn logged_missed_wins_regardless_of_now() {
        let w = window();
        let now = w.start + Duration::minutes(5);
        assert_eq!(
            compute_state(now, &w, Some((LoggedStatus::Missed, now))),
            PrayerState::Missed
        );
    }
}
