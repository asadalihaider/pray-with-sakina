use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Serialize;

use crate::engine::{
    first_nudge_time, next_nudge, nudge_cutoff, window_for, LoggedStatus, Prayer, PrayerCalcError,
};
use crate::settings::Settings;
use crate::store::PrayerLog;
use crate::today;

/// What the floating card is told to show.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DueNudge {
    pub prayer: Prayer,
    pub label: &'static str,
    pub arabic: &'static str,
    pub remaining_ms: i64,
    pub window_fraction: f64,
    /// Under 20 minutes left: the calmer "ending soon" treatment.
    pub ending_soon: bool,
}

/// Decides when a reminder is due. The record of what has already fired
/// lives in the database rather than here, so restarting the app mid-window
/// does not repeat a reminder the user has already seen.
#[derive(Default)]
pub struct Scheduler;

fn bedtime_on(date: NaiveDate, settings: &Settings) -> DateTime<Utc> {
    let (hour, minute) = settings.bedtime();
    let local = date.and_time(
        NaiveTime::from_hms_opt(hour, minute, 0).unwrap_or_else(|| {
            NaiveTime::from_hms_opt(23, 30, 0).expect("23:30 is a valid time")
        }),
    );
    settings
        .timezone()
        .from_local_datetime(&local)
        .single()
        // A bedtime that falls in a DST gap: push past it rather than
        // dropping the cutoff entirely.
        .map(|resolved| resolved.with_timezone(&Utc))
        .unwrap_or_else(|| {
            settings
                .timezone()
                .from_local_datetime(&(local + Duration::hours(1)))
                .earliest()
                .map(|resolved| resolved.with_timezone(&Utc))
                .unwrap_or_else(|| Utc.from_utc_datetime(&local))
        })
}

impl Scheduler {
    /// Returns a nudge if one is due at `now`, and records it so the next
    /// one is scheduled from it.
    ///
    /// Only ever returns a single nudge, even when many were missed while
    /// the laptop slept: the schedule is fast-forwarded to the most recent
    /// due nudge so waking up gives one reminder, not a burst of them.
    pub fn poll(
        &self,
        now: DateTime<Utc>,
        settings: &Settings,
        log: &PrayerLog,
    ) -> Result<Option<DueNudge>, PrayerCalcError> {
        let date = today::display_date(now, settings)?;
        let day = today::day_times_for(date, settings)?;

        let Some(prayer) = Prayer::ALL
            .into_iter()
            .find(|prayer| window_for(&day, *prayer).contains(now))
        else {
            return Ok(None);
        };

        if matches!(log.get(date, prayer), Some((LoggedStatus::Prayed, _))) {
            return Ok(None);
        }

        let window = window_for(&day, prayer);
        let cutoff = nudge_cutoff(prayer, &window, bedtime_on(date, settings));
        let friday = date.weekday() == chrono::Weekday::Fri;
        let first = first_nudge_time(
            &window,
            settings.first_nudge_for(prayer, friday),
            settings.timezone(),
        );

        if now < first || now >= cutoff {
            return Ok(None);
        }

        let due = match log.last_reminder(date, prayer) {
            None => Some(first),
            Some(last) => {
                let mut cursor = last;
                let mut due = None;
                while let Some(event) = next_nudge(cursor, cutoff) {
                    if event.at > now {
                        break;
                    }
                    due = Some(event.at);
                    cursor = event.at;
                }
                due
            }
        };

        let Some(at) = due else {
            return Ok(None);
        };
        log.record_reminder(date, prayer, at);

        // The card counts down the prayer window, not the nudge cutoff, so
        // Isha still reads as time left to pray rather than time to bedtime.
        let remaining = window.end - now;
        let total = window.end - window.start;

        Ok(Some(DueNudge {
            prayer,
            label: today::label_for(prayer, date),
            arabic: today::arabic_for(prayer, date),
            remaining_ms: remaining.num_milliseconds(),
            window_fraction: remaining.num_milliseconds() as f64
                / total.num_milliseconds().max(1) as f64,
            ending_soon: remaining < Duration::minutes(20),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::LoggedStatus;

    /// 2026-06-15 in Gujranwala: Zuhr runs 12:01-16:24 local (07:01-11:24
    /// UTC), so the +60 minute first nudge lands at 08:01 UTC.
    fn zuhr_first_nudge() -> DateTime<Utc> {
        let settings = Settings::for_tests();
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let day = today::day_times_for(date, &settings).unwrap();
        first_nudge_time(
            &window_for(&day, Prayer::Zuhr),
            settings.first_nudge_for(Prayer::Zuhr, false),
            settings.timezone(),
        )
    }

    #[test]
    fn nothing_fires_before_the_first_nudge_offset() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let before = zuhr_first_nudge() - Duration::minutes(1);
        assert_eq!(
            scheduler.poll(before, &settings, &PrayerLog::in_memory().unwrap()).unwrap(),
            None
        );
    }

    #[test]
    fn the_first_nudge_fires_once_then_waits_for_its_interval() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let first = zuhr_first_nudge();

        let due = scheduler.poll(first, &settings, &log).unwrap();
        assert!(due.is_some());
        assert_eq!(due.unwrap().prayer, Prayer::Zuhr);

        // A tick 30 seconds later must not fire again.
        assert_eq!(
            scheduler
                .poll(first + Duration::seconds(30), &settings, &log)
                .unwrap(),
            None
        );

        // The Zuhr window still has hours left, so the next one is +30 min.
        assert!(scheduler
            .poll(first + Duration::minutes(30), &settings, &log)
            .unwrap()
            .is_some());
    }

    #[test]
    fn restarting_the_app_does_not_repeat_a_reminder() {
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let first = zuhr_first_nudge();

        // The reminder fires under one scheduler...
        assert!(Scheduler::default().poll(first, &settings, &log).unwrap().is_some());

        // ...and a fresh one, as after a restart, must not fire it again.
        let after_restart = Scheduler::default();
        assert_eq!(
            after_restart
                .poll(first + Duration::seconds(30), &settings, &log)
                .unwrap(),
            None
        );

        // The schedule still advances normally from where it left off.
        assert!(after_restart
            .poll(first + Duration::minutes(30), &settings, &log)
            .unwrap()
            .is_some());
    }

    #[test]
    fn logging_the_prayer_stops_the_nudges() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let first = zuhr_first_nudge();
        let date = today::display_date(first, &settings).unwrap();

        log.set(date, Prayer::Zuhr, LoggedStatus::Prayed, first);
        assert_eq!(scheduler.poll(first, &settings, &log).unwrap(), None);
    }

    #[test]
    fn sleeping_through_many_nudges_wakes_to_a_single_one() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let first = zuhr_first_nudge();

        scheduler.poll(first, &settings, &log).unwrap();

        // Two hours later several nudges were missed; exactly one fires.
        let woke = first + Duration::hours(2);
        assert!(scheduler.poll(woke, &settings, &log).unwrap().is_some());
        assert_eq!(
            scheduler
                .poll(woke + Duration::seconds(30), &settings, &log)
                .unwrap(),
            None
        );
    }

    #[test]
    fn isha_nudges_stop_at_bedtime() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        // 2026-06-15, 23:45 local == 18:45 UTC: inside the Isha window but
        // past the 23:30 bedtime.
        let after_bedtime = Utc.with_ymd_and_hms(2026, 6, 15, 18, 45, 0).unwrap();
        assert_eq!(
            scheduler.poll(after_bedtime, &settings, &log).unwrap(),
            None
        );
    }

    #[test]
    fn a_nudge_close_to_the_window_end_is_marked_ending_soon() {
        let settings = Settings::for_tests();
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let day = today::day_times_for(date, &settings).unwrap();
        let window = window_for(&day, Prayer::Zuhr);

        let scheduler = Scheduler::default();
        let log = PrayerLog::in_memory().unwrap();
        let nearly_over = window.end - Duration::minutes(10);

        let due = scheduler.poll(nearly_over, &settings, &log).unwrap().unwrap();
        assert!(due.ending_soon);
    }
}
