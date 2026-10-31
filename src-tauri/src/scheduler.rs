use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};
use serde::Serialize;

use crate::engine::{
    first_nudge_time, next_nudge, nudge_cutoff, window_for, FirstNudge, LoggedStatus, Prayer,
    PrayerCalcError,
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

/// A prayer's time has just begun.
#[derive(Debug, Clone, PartialEq)]
pub struct StartNotice {
    pub prayer: Prayer,
    pub label: &'static str,
    /// The local jamaat time, when one is set for this prayer and is still
    /// to come, so the notice can say how long there is to get ready.
    pub jamaat: Option<NaiveTime>,
}

/// How long after a window opens its start can still be announced.
///
/// Wide enough to survive the scheduler's 30-second tick and a Mac that
/// woke a minute late; narrow enough that opening the lid at four in the
/// afternoon does not announce a Zuhr that began at one. Telling someone a
/// prayer "has started" hours after it did is worse than saying nothing.
const START_GRACE: Duration = Duration::minutes(2);

/// How long after Isha begins the Surah Mulk reminders start, when Isha has
/// not been logged as prayed by then.
const MULK_AFTER_ISHA: Duration = Duration::hours(1);
/// How soon after Isha is logged as prayed they start instead, if sooner.
const MULK_AFTER_PRAYED: Duration = Duration::minutes(15);
/// How often they repeat until Mulk is logged as recited.
const MULK_INTERVAL: Duration = Duration::minutes(30);
/// On a night too short for the usual start, the single reminder comes
/// this long before bedtime.
const MULK_BEFORE_BEDTIME: Duration = Duration::minutes(30);

/// Decides when a reminder is due. The record of what has already fired
/// lives in the database rather than here, so restarting the app mid-window
/// does not repeat a reminder the user has already seen.
#[derive(Default)]
pub struct Scheduler;

/// The bedtime that ends the night of `date`.
///
/// A bedtime before noon is the early hours after that night, not the
/// morning of the same day: 00:30 means half past midnight, after Isha.
/// Read as the same day, it fell before Isha had even begun and silenced
/// every reminder that night.
fn bedtime_on(date: NaiveDate, settings: &Settings) -> DateTime<Utc> {
    let (hour, minute) = settings.bedtime();
    let night = if hour < 12 {
        date + Duration::days(1)
    } else {
        date
    };
    let local = night.and_time(
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
    /// Returns a start notice if a prayer's window has just opened and it
    /// has not been announced yet, recording it so it is announced once.
    ///
    /// Separate from `poll` because it answers a different question: not
    /// "have you prayed?" but "the time has come". It ignores the first
    /// nudge offset, since the whole point is the moment the window opens.
    pub fn poll_start(
        &self,
        now: DateTime<Utc>,
        settings: &Settings,
        log: &PrayerLog,
    ) -> Result<Option<StartNotice>, PrayerCalcError> {
        if !settings.announce_start {
            return Ok(None);
        }

        let date = today::display_date(now, settings)?;
        let day = today::day_times_for(date, settings)?;

        let Some(prayer) = Prayer::ALL
            .into_iter()
            .find(|prayer| window_for(&day, *prayer).contains(now))
        else {
            return Ok(None);
        };
        let window = window_for(&day, prayer);

        if now - window.start > START_GRACE {
            return Ok(None);
        }
        // Isha past bedtime gets no nudges, and no announcement either.
        if now >= nudge_cutoff(prayer, &window, bedtime_on(date, settings)) {
            return Ok(None);
        }
        if log.start_announced(date, prayer) {
            return Ok(None);
        }

        let friday = date.weekday() == chrono::Weekday::Fri;
        let jamaat = match settings.first_nudge_for(prayer, friday) {
            config @ FirstNudge::JamaatTime { .. } => {
                let at = first_nudge_time(&window, config, settings.timezone());
                // A jamaat time already behind us says nothing useful.
                (at > now).then(|| at.with_timezone(&settings.timezone()).time())
            }
            FirstNudge::OffsetMinutes { .. } => None,
        };

        log.record_start_announced(date, prayer, now);
        Ok(Some(StartNotice {
            prayer,
            label: today::label_for(prayer, date),
            jamaat,
        }))
    }

    /// Whether a Surah Mulk reminder is due at `now`, recording it if so.
    ///
    /// Mulk is recited once a night, after whichever prayer suits, so the
    /// reminders wait until the day's prayers are done: an hour after Isha
    /// begins, or 15 minutes after Isha is logged as prayed if that comes
    /// first. A missed Isha does not bring them forward. They repeat every
    /// 30 minutes until Mulk is logged as recited, and stop at bedtime.
    ///
    /// Logged as not recited does not stop them, for the same reason a
    /// missed prayer is still nudged while its window is open: there is
    /// still time.
    pub fn poll_mulk(
        &self,
        now: DateTime<Utc>,
        settings: &Settings,
        log: &PrayerLog,
    ) -> Result<bool, PrayerCalcError> {
        if !settings.recite_mulk {
            return Ok(false);
        }

        let date = today::display_date(now, settings)?;
        if log.mulk_recited(date) {
            return Ok(false);
        }

        let day = today::day_times_for(date, settings)?;
        let isha = window_for(&day, Prayer::Isha);
        let bedtime = bedtime_on(date, settings);

        let mut start = isha.start + MULK_AFTER_ISHA;
        if let Some((LoggedStatus::Prayed, at)) = log.get(date, Prayer::Isha) {
            start = start.min(at + MULK_AFTER_PRAYED);
        }
        // A night too short for that still gets one reminder, but never
        // before Isha has begun.
        if start >= bedtime {
            start = (bedtime - MULK_BEFORE_BEDTIME).max(isha.start);
        }

        if now < start || now >= bedtime {
            return Ok(false);
        }

        // Fast-forward to the most recent due reminder, so waking the Mac
        // gives one, not a burst of the ones it slept through.
        let due = match log.last_mulk_reminder(date) {
            None => start,
            Some(last) => {
                let missed = (now - last).num_seconds() / MULK_INTERVAL.num_seconds();
                if missed < 1 {
                    return Ok(false);
                }
                last + MULK_INTERVAL * missed as i32
            }
        };

        log.record_mulk_reminder(date, due);
        Ok(true)
    }

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
    use chrono::Timelike;

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

    /// When Zuhr's window opens on a given day in Gujranwala.
    fn zuhr_opens_on(date: NaiveDate) -> DateTime<Utc> {
        let settings = Settings::for_tests();
        let day = today::day_times_for(date, &settings).unwrap();
        window_for(&day, Prayer::Zuhr).start
    }

    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()
    }

    /// Settings with Zuhr's reminder at a fixed jamaat time.
    fn zuhr_jamaat_at(hour: u32, minute: u32) -> Settings {
        let mut settings = Settings::for_tests();
        for setting in settings.first_nudge.iter_mut() {
            if setting.prayer == Prayer::Zuhr {
                setting.first_nudge = FirstNudge::JamaatTime { hour, minute };
            }
        }
        settings
    }

    #[test]
    fn the_start_notice_carries_a_jamaat_time_still_to_come() {
        let settings = zuhr_jamaat_at(13, 30);
        let log = PrayerLog::in_memory().unwrap();
        let opened = zuhr_opens_on(monday());

        let notice = Scheduler::default()
            .poll_start(opened + Duration::seconds(20), &settings, &log)
            .unwrap()
            .expect("the opening is announced");
        assert_eq!(notice.jamaat, NaiveTime::from_hms_opt(13, 30, 0));
    }

    #[test]
    fn the_start_notice_leaves_out_a_jamaat_time_already_past() {
        // Zuhr opens at 12:01 here, so a 12:00 jamaat has been and gone.
        let settings = zuhr_jamaat_at(12, 0);
        let log = PrayerLog::in_memory().unwrap();
        let opened = zuhr_opens_on(monday());

        let notice = Scheduler::default()
            .poll_start(opened + Duration::seconds(20), &settings, &log)
            .unwrap()
            .expect("the opening is announced");
        assert_eq!(notice.jamaat, None);
    }

    #[test]
    fn the_start_notice_has_no_jamaat_time_without_one_set() {
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let opened = zuhr_opens_on(monday());

        let notice = Scheduler::default()
            .poll_start(opened + Duration::seconds(20), &settings, &log)
            .unwrap()
            .expect("the opening is announced");
        assert_eq!(notice.jamaat, None);
    }

    #[test]
    fn the_opening_of_a_window_is_announced_once() {
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let opened = zuhr_opens_on(monday());
        let scheduler = Scheduler::default();

        let notice = scheduler
            .poll_start(opened + Duration::seconds(20), &settings, &log)
            .unwrap()
            .expect("the opening is announced");
        assert_eq!(notice.prayer, Prayer::Zuhr);
        assert_eq!(notice.label, "Zuhr");

        // The next tick, thirty seconds on, says nothing more.
        assert_eq!(
            scheduler
                .poll_start(opened + Duration::seconds(50), &settings, &log)
                .unwrap(),
            None
        );
    }

    #[test]
    fn a_restart_does_not_announce_the_same_opening_twice() {
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let opened = zuhr_opens_on(monday());

        assert!(Scheduler::default()
            .poll_start(opened, &settings, &log)
            .unwrap()
            .is_some());
        // A fresh scheduler, as after relaunching, reads the record rather
        // than its own memory.
        assert_eq!(
            Scheduler::default()
                .poll_start(opened + Duration::seconds(30), &settings, &log)
                .unwrap(),
            None
        );
    }

    #[test]
    fn waking_long_after_a_window_opened_announces_nothing() {
        // The lid opened at four in the afternoon. "Zuhr has started" would
        // be telling the user something three hours out of date.
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let opened = zuhr_opens_on(monday());

        assert_eq!(
            Scheduler::default()
                .poll_start(opened + Duration::hours(3), &settings, &log)
                .unwrap(),
            None
        );
        // Nor is it marked as announced, should the record matter later.
        assert!(!log.start_announced(monday(), Prayer::Zuhr));
    }

    #[test]
    fn announcing_can_be_switched_off() {
        let mut settings = Settings::for_tests();
        settings.announce_start = false;
        let log = PrayerLog::in_memory().unwrap();

        assert_eq!(
            Scheduler::default()
                .poll_start(zuhr_opens_on(monday()), &settings, &log)
                .unwrap(),
            None
        );
    }

    #[test]
    fn friday_announces_jumuah() {
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let friday = NaiveDate::from_ymd_opt(2026, 6, 19).unwrap();

        let notice = Scheduler::default()
            .poll_start(zuhr_opens_on(friday), &settings, &log)
            .unwrap()
            .expect("Friday's Zuhr is announced");
        assert_ne!(notice.label, "Zuhr", "it should be called Jumu'ah");
    }

    #[test]
    fn an_announcement_does_not_move_the_nudges() {
        // Kept in its own table for exactly this: the nudge schedule counts
        // forward from the last recorded reminder.
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        Scheduler::default()
            .poll_start(zuhr_opens_on(monday()), &settings, &log)
            .unwrap();

        assert_eq!(log.last_reminder(monday(), Prayer::Zuhr), None);
        assert!(Scheduler::default()
            .poll(zuhr_first_nudge(), &settings, &log)
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

    /// When Isha's window opens on Monday in Gujranwala, well before the
    /// default 23:30 bedtime.
    fn isha_opens() -> DateTime<Utc> {
        let settings = Settings::for_tests();
        let day = today::day_times_for(monday(), &settings).unwrap();
        window_for(&day, Prayer::Isha).start
    }

    #[test]
    fn mulk_waits_an_hour_after_isha_begins() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();

        let early = isha_opens() + Duration::minutes(59);
        assert!(!scheduler.poll_mulk(early, &settings, &log).unwrap());
        let on_time = isha_opens() + Duration::hours(1);
        assert!(scheduler.poll_mulk(on_time, &settings, &log).unwrap());
    }

    #[test]
    fn praying_isha_brings_mulk_forward() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let prayed = isha_opens() + Duration::minutes(5);
        log.set(monday(), Prayer::Isha, LoggedStatus::Prayed, prayed);

        let too_soon = prayed + Duration::minutes(14);
        assert!(!scheduler.poll_mulk(too_soon, &settings, &log).unwrap());
        let after = prayed + Duration::minutes(15);
        assert!(scheduler.poll_mulk(after, &settings, &log).unwrap());
    }

    #[test]
    fn a_missed_isha_does_not_bring_mulk_forward() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let missed = isha_opens() + Duration::minutes(5);
        log.set(monday(), Prayer::Isha, LoggedStatus::Missed, missed);

        let later = missed + Duration::minutes(15);
        assert!(!scheduler.poll_mulk(later, &settings, &log).unwrap());
    }

    #[test]
    fn mulk_repeats_every_half_hour() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let first = isha_opens() + Duration::hours(1);

        assert!(scheduler.poll_mulk(first, &settings, &log).unwrap());
        let tick = first + Duration::seconds(30);
        assert!(!scheduler.poll_mulk(tick, &settings, &log).unwrap());
        let almost = first + Duration::minutes(29);
        assert!(!scheduler.poll_mulk(almost, &settings, &log).unwrap());
        let next = first + Duration::minutes(30);
        assert!(scheduler.poll_mulk(next, &settings, &log).unwrap());
    }

    #[test]
    fn only_recited_stops_the_mulk_reminders() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let first = isha_opens() + Duration::hours(1);

        // Not recited yet, but bedtime is still to come: keep asking.
        log.set_mulk(monday(), false, first);
        assert!(scheduler.poll_mulk(first, &settings, &log).unwrap());

        log.set_mulk(monday(), true, first);
        let next = first + Duration::minutes(30);
        assert!(!scheduler.poll_mulk(next, &settings, &log).unwrap());
    }

    #[test]
    fn mulk_reminders_stop_at_bedtime() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let bedtime = bedtime_on(monday(), &settings);

        assert!(!scheduler.poll_mulk(bedtime, &settings, &log).unwrap());
        let before = bedtime - Duration::minutes(1);
        assert!(scheduler.poll_mulk(before, &settings, &log).unwrap());
    }

    #[test]
    fn a_short_night_gets_one_mulk_reminder_before_bedtime() {
        let scheduler = Scheduler::default();
        let mut settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        // Bedtime 40 minutes after Isha begins: too soon for the usual hour.
        let bedtime = (isha_opens() + Duration::minutes(40)).with_timezone(&settings.timezone());
        settings.bedtime_hour = bedtime.hour();
        settings.bedtime_minute = bedtime.minute();
        let bedtime = bedtime_on(monday(), &settings);

        let early = bedtime - Duration::minutes(31);
        assert!(!scheduler.poll_mulk(early, &settings, &log).unwrap());
        let once = bedtime - Duration::minutes(30);
        assert!(scheduler.poll_mulk(once, &settings, &log).unwrap());
        let late = bedtime - Duration::minutes(1);
        assert!(!scheduler.poll_mulk(late, &settings, &log).unwrap());
    }

    #[test]
    fn waking_late_gives_one_mulk_reminder_not_a_burst() {
        let scheduler = Scheduler::default();
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let first = isha_opens() + Duration::hours(1);
        assert!(scheduler.poll_mulk(first, &settings, &log).unwrap());

        let woke = first + Duration::minutes(65);
        assert!(scheduler.poll_mulk(woke, &settings, &log).unwrap());
        let tick = woke + Duration::seconds(30);
        assert!(!scheduler.poll_mulk(tick, &settings, &log).unwrap());
    }

    #[test]
    fn no_mulk_reminders_when_it_is_not_tracked() {
        let scheduler = Scheduler::default();
        let mut settings = Settings::for_tests();
        settings.recite_mulk = false;
        let log = PrayerLog::in_memory().unwrap();

        let first = isha_opens() + Duration::hours(1);
        assert!(!scheduler.poll_mulk(first, &settings, &log).unwrap());
    }

    #[test]
    fn a_bedtime_after_midnight_belongs_to_the_night_before() {
        let mut settings = Settings::for_tests();
        settings.bedtime_hour = 0;
        settings.bedtime_minute = 30;

        let bedtime = bedtime_on(monday(), &settings).with_timezone(&settings.timezone());
        assert_eq!(bedtime.date_naive(), monday() + Duration::days(1));
        assert_eq!((bedtime.hour(), bedtime.minute()), (0, 30));
    }

    #[test]
    fn an_evening_bedtime_stays_on_the_same_day() {
        let settings = Settings::for_tests();
        let bedtime = bedtime_on(monday(), &settings).with_timezone(&settings.timezone());
        assert_eq!(bedtime.date_naive(), monday());
        assert_eq!((bedtime.hour(), bedtime.minute()), (23, 30));
    }

    #[test]
    fn a_bedtime_after_midnight_keeps_the_night_reminders() {
        let scheduler = Scheduler::default();
        let mut settings = Settings::for_tests();
        settings.bedtime_hour = 0;
        settings.bedtime_minute = 30;
        let log = PrayerLog::in_memory().unwrap();

        // Past the default 23:30 and still before half past midnight.
        let late = bedtime_on(monday(), &settings) - Duration::minutes(10);
        assert!(scheduler.poll(late, &settings, &log).unwrap().is_some());
        assert!(scheduler.poll_mulk(late, &settings, &log).unwrap());
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
