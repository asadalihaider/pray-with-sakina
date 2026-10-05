use chrono::{DateTime, Utc};

use super::times::DayTimes;
use super::types::Prayer;

/// The span during which a prayer can be prayed on time. See build plan
/// section 4.1: each prayer's window ends at the start of the next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Window {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Window {
    pub fn contains(&self, at: DateTime<Utc>) -> bool {
        at >= self.start && at < self.end
    }
}

/// Computes every prayer's window for one calendar day. Isha's window ends
/// at the next Fajr (`day.next_fajr`); the bedtime cutoff only affects when
/// nudges stop firing, not the window itself (see `nudges::nudge_cutoff`).
pub fn compute_windows(day: &DayTimes) -> [(Prayer, Window); 5] {
    [
        (
            Prayer::Fajr,
            Window {
                start: day.fajr,
                end: day.sunrise,
            },
        ),
        (
            Prayer::Zuhr,
            Window {
                start: day.zuhr,
                end: day.asr,
            },
        ),
        (
            Prayer::Asr,
            Window {
                start: day.asr,
                end: day.maghrib,
            },
        ),
        (
            Prayer::Maghrib,
            Window {
                start: day.maghrib,
                end: day.isha,
            },
        ),
        (
            Prayer::Isha,
            Window {
                start: day.isha,
                end: day.next_fajr,
            },
        ),
    ]
}

pub fn window_for(day: &DayTimes, prayer: Prayer) -> Window {
    let windows = compute_windows(day);
    windows
        .into_iter()
        .find(|(p, _)| *p == prayer)
        .map(|(_, w)| w)
        .expect("Prayer::ALL covers every variant")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn sample_day() -> DayTimes {
        let base = Utc.with_ymd_and_hms(2026, 6, 15, 0, 0, 0).unwrap();
        DayTimes {
            fajr: base + Duration::hours(4),
            sunrise: base + Duration::hours(5) + Duration::minutes(30),
            zuhr: base + Duration::hours(12) + Duration::minutes(15),
            asr: base + Duration::hours(16),
            maghrib: base + Duration::hours(19) + Duration::minutes(10),
            isha: base + Duration::hours(20) + Duration::minutes(40),
            next_fajr: base + Duration::days(1) + Duration::hours(4) + Duration::minutes(2),
        }
    }

    #[test]
    fn each_window_ends_where_the_next_begins() {
        let day = sample_day();
        assert_eq!(window_for(&day, Prayer::Fajr).end, day.sunrise);
        assert_eq!(window_for(&day, Prayer::Zuhr).start, day.zuhr);
        assert_eq!(window_for(&day, Prayer::Zuhr).end, day.asr);
        assert_eq!(window_for(&day, Prayer::Asr).end, day.maghrib);
        assert_eq!(window_for(&day, Prayer::Maghrib).end, day.isha);
    }

    #[test]
    fn isha_window_runs_past_midnight_into_next_fajr() {
        let day = sample_day();
        let isha = window_for(&day, Prayer::Isha);
        assert_eq!(isha.end, day.next_fajr);

        let one_am_next_day = day.isha + Duration::hours(5);
        assert!(isha.contains(one_am_next_day));
    }
}
