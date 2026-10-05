use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;

use super::types::{FirstNudge, Prayer};
use super::windows::Window;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NudgeUrgency {
    Normal,
    EndingSoon,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NudgeEvent {
    pub at: DateTime<Utc>,
    pub urgency: NudgeUrgency,
}

/// Build plan section 4.3: the repeat interval depends on how much time is
/// left in the window at the moment a nudge fires, not on elapsed time.
fn interval_for_remaining(remaining: Duration) -> (Duration, NudgeUrgency) {
    if remaining > Duration::minutes(60) {
        (Duration::minutes(30), NudgeUrgency::Normal)
    } else if remaining > Duration::minutes(20) {
        (Duration::minutes(15), NudgeUrgency::Normal)
    } else {
        (Duration::minutes(5), NudgeUrgency::EndingSoon)
    }
}

/// The first nudge time for a prayer: either a fixed offset after the
/// window starts, or an exact jamaat/Jumu'ah time on the window's calendar
/// day, resolved in the user's local timezone.
pub fn first_nudge_time(window: &Window, config: FirstNudge, tz: Tz) -> DateTime<Utc> {
    match config {
        FirstNudge::OffsetMinutes { minutes } => window.start + Duration::minutes(minutes),
        FirstNudge::JamaatTime { hour, minute } => {
            let local_date = window.start.with_timezone(&tz).date_naive();
            let local_time = NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_else(|| {
                window.start.with_timezone(&tz).time()
            });
            match tz.from_local_datetime(&local_date.and_time(local_time)).single() {
                Some(dt) => dt.with_timezone(&Utc),
                // Ambiguous/nonexistent local time (DST transition): fall
                // back to the offset-free window start rather than panic.
                None => window.start,
            }
        }
    }
}

/// Given the last nudge that fired (or the first nudge time, before any
/// have), returns the next one, or `None` once too little of the window is
/// left to warrant another.
pub fn next_nudge(last_nudge: DateTime<Utc>, window_end: DateTime<Utc>) -> Option<NudgeEvent> {
    if last_nudge >= window_end {
        return None;
    }
    let remaining = window_end - last_nudge;
    let (interval, _) = interval_for_remaining(remaining);
    let next_at = last_nudge + interval;
    if next_at >= window_end {
        return None;
    }
    let next_remaining = window_end - next_at;
    let (_, urgency) = interval_for_remaining(next_remaining);
    Some(NudgeEvent {
        at: next_at,
        urgency,
    })
}

/// Isha nudges stop at bedtime even though the window itself runs until the
/// next Fajr; every other prayer's cutoff is just its window end.
pub fn nudge_cutoff(prayer: Prayer, window: &Window, bedtime: DateTime<Utc>) -> DateTime<Utc> {
    if prayer == Prayer::Isha {
        window.end.min(bedtime)
    } else {
        window.end
    }
}

/// Expands a full nudge schedule from the first nudge to the cutoff. The
/// live scheduler calls `next_nudge` once per tick against real elapsed
/// time instead, so this exists to make the shape of a whole schedule
/// assertable in one go.
#[cfg(test)]
pub fn nudge_schedule(first_nudge: DateTime<Utc>, cutoff: DateTime<Utc>) -> Vec<NudgeEvent> {
    let mut events = Vec::new();
    if first_nudge >= cutoff {
        return events;
    }
    let remaining = cutoff - first_nudge;
    let (_, urgency) = interval_for_remaining(remaining);
    events.push(NudgeEvent {
        at: first_nudge,
        urgency,
    });

    let mut last = first_nudge;
    while let Some(event) = next_nudge(last, cutoff) {
        events.push(event);
        last = event.at;
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn more_than_an_hour_left_nudges_every_thirty_minutes() {
        let (interval, urgency) = interval_for_remaining(Duration::minutes(90));
        assert_eq!(interval, Duration::minutes(30));
        assert_eq!(urgency, NudgeUrgency::Normal);
    }

    #[test]
    fn exactly_sixty_minutes_left_falls_into_the_fifteen_minute_tier() {
        let (interval, _) = interval_for_remaining(Duration::minutes(60));
        assert_eq!(interval, Duration::minutes(15));
    }

    #[test]
    fn exactly_twenty_minutes_left_falls_into_the_five_minute_tier() {
        let (interval, urgency) = interval_for_remaining(Duration::minutes(20));
        assert_eq!(interval, Duration::minutes(5));
        assert_eq!(urgency, NudgeUrgency::EndingSoon);
    }

    #[test]
    fn schedule_gets_more_frequent_as_the_window_closes() {
        let start = Utc.with_ymd_and_hms(2026, 6, 15, 12, 0, 0).unwrap();
        let window_end = start + Duration::minutes(130); // Zuhr-sized window
        let first = start + Duration::minutes(60); // +60 offset, matches default

        let schedule = nudge_schedule(first, window_end);
        let gaps: Vec<Duration> = schedule
            .windows(2)
            .map(|pair| pair[1].at - pair[0].at)
            .collect();

        // 70 min left at first nudge -> 30 min tier, then tightens.
        assert_eq!(gaps[0], Duration::minutes(30));
        assert!(gaps.last().unwrap() <= &Duration::minutes(15));
        assert!(schedule.iter().all(|e| e.at < window_end));
    }

    #[test]
    fn last_nudge_past_window_end_yields_none() {
        let end = Utc.with_ymd_and_hms(2026, 6, 15, 13, 0, 0).unwrap();
        assert_eq!(next_nudge(end, end), None);
        assert_eq!(next_nudge(end + Duration::minutes(1), end), None);
    }

    #[test]
    fn isha_cutoff_is_bedtime_even_though_window_runs_to_next_fajr() {
        let isha_start = Utc.with_ymd_and_hms(2026, 6, 15, 20, 40, 0).unwrap();
        let next_fajr = Utc.with_ymd_and_hms(2026, 6, 16, 4, 2, 0).unwrap();
        let window = Window {
            start: isha_start,
            end: next_fajr,
        };
        let bedtime = Utc.with_ymd_and_hms(2026, 6, 15, 23, 30, 0).unwrap();

        assert_eq!(nudge_cutoff(Prayer::Isha, &window, bedtime), bedtime);
        assert_eq!(nudge_cutoff(Prayer::Maghrib, &window, bedtime), window.end);
    }

    #[test]
    fn jamaat_time_resolves_in_the_local_timezone() {
        let tz: Tz = "Asia/Karachi".parse().unwrap(); // UTC+5, no DST
        let window = Window {
            start: Utc.with_ymd_and_hms(2026, 6, 15, 7, 0, 0).unwrap(),
            end: Utc.with_ymd_and_hms(2026, 6, 15, 8, 0, 0).unwrap(),
        };
        let at = first_nudge_time(
            &window,
            FirstNudge::JamaatTime {
                hour: 13,
                minute: 15,
            },
            tz,
        );
        // 13:15 PKT (UTC+5) == 08:15 UTC.
        assert_eq!(at, Utc.with_ymd_and_hms(2026, 6, 15, 8, 15, 0).unwrap());
    }
}
