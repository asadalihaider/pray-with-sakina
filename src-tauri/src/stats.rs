use std::collections::HashMap;

use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use serde::Serialize;

use crate::engine::{window_for, LoggedStatus, Prayer, PrayerCalcError};
use crate::settings::Settings;
use crate::store::PrayerLog;
use crate::today;

/// How far back a streak is worth counting. A streak longer than a year is
/// not worth the calendar arithmetic to prove.
const STREAK_LIMIT: i64 = 366;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsView {
    pub label: String,
    pub year: i32,
    pub month: u32,
    pub days: Vec<DayStats>,
    pub streak: u32,
    /// None when Surah Mulk tracking is switched off.
    pub mulk_streak: Option<u32>,
    /// Share of this month's finished prayers that were logged as prayed.
    pub on_time: Option<u32>,
    /// False for the current month, so the user cannot page into the future.
    pub has_next: bool,
    /// Today in the user's own timezone, so the grid can tell a day that has
    /// happened from one that has not without guessing from the browser's
    /// clock.
    pub today: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayStats {
    pub date: String,
    pub day: u32,
    pub statuses: Vec<&'static str>,
    /// Kept beside the prayers rather than inside them, so a Mulk entry can
    /// never be mistaken for a sixth prayer.
    pub mulk: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QazaView {
    /// Prayers still owed.
    pub total: i64,
    /// Prayers made up, counted only up to what was owed.
    pub made_up: i64,
    pub rows: Vec<QazaRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QazaRow {
    pub prayer: Prayer,
    pub label: &'static str,
    pub remaining: i64,
    /// Made up so far, capped at what was owed, so that made up and
    /// remaining always add up to the debt the bar is drawn against. Making
    /// up more than that is possible — lowering a backlog after the fact —
    /// but there is nothing for the extra to count towards.
    pub made_up: i64,
    pub backlog: i64,
}

fn status_name(status: Option<LoggedStatus>) -> &'static str {
    match status {
        Some(LoggedStatus::Prayed) => "prayed",
        Some(LoggedStatus::Missed) => "missed",
        None => "unlogged",
    }
}

fn last_day_of(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|first| first.pred_opt())
        .map(|last| last.day())
        .unwrap_or(28)
}

/// Whether every prayer that day was logged as prayed — the bar a day has to
/// clear to extend a streak.
fn all_prayed(date: NaiveDate, logs: &HashMap<(NaiveDate, Prayer), LoggedStatus>) -> bool {
    Prayer::ALL
        .into_iter()
        .all(|prayer| logs.get(&(date, prayer)) == Some(&LoggedStatus::Prayed))
}

pub fn month_stats(
    now: DateTime<Utc>,
    settings: &Settings,
    log: &PrayerLog,
    year: i32,
    month: u32,
) -> Result<StatsView, PrayerCalcError> {
    let today = today::display_date(now, settings)?;
    let last_day = last_day_of(year, month);
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return Err(PrayerCalcError::UnsupportedLocation);
    };
    let last = NaiveDate::from_ymd_opt(year, month, last_day).unwrap_or(first);

    let logs: HashMap<(NaiveDate, Prayer), LoggedStatus> = log
        .logs_between(first, last)
        .into_iter()
        .map(|(date, prayer, status)| ((date, prayer), status))
        .collect();

    let mulk: HashMap<NaiveDate, bool> = log.mulk_between(first, last).into_iter().collect();

    let mut days = Vec::with_capacity(last_day as usize);
    let mut prayed = 0u32;
    let mut finished = 0u32;

    for day in 1..=last_day {
        let Some(date) = NaiveDate::from_ymd_opt(year, month, day) else {
            continue;
        };
        let times = today::day_times_for(date, settings).ok();

        let statuses = Prayer::ALL
            .into_iter()
            .map(|prayer| {
                let logged = logs.get(&(date, prayer)).copied();
                // A window that has not closed yet is not a miss, it simply
                // has not happened.
                let closed = times
                    .as_ref()
                    .map(|times| window_for(times, prayer).end <= now)
                    .unwrap_or(false);

                if logged.is_none() && !closed {
                    return "upcoming";
                }
                finished += 1;
                if logged == Some(LoggedStatus::Prayed) {
                    prayed += 1;
                }
                status_name(logged)
            })
            .collect();

        let mulk_status = settings.recite_mulk.then(|| match mulk.get(&date) {
            Some(true) => "prayed",
            Some(false) => "missed",
            // A day still running has not been missed yet.
            None if date >= today => "upcoming",
            None => "unlogged",
        });

        days.push(DayStats {
            date: date.to_string(),
            day,
            statuses,
            mulk: mulk_status,
        });
    }

    let mut streak = 0u32;
    let mut cursor = today;
    // Today only breaks a streak once it is over; until then it is still in
    // progress, so counting starts from the last completed day.
    if !all_prayed(cursor, &log_map(log, cursor, cursor)) {
        cursor -= Duration::days(1);
    }
    for _ in 0..STREAK_LIMIT {
        if all_prayed(cursor, &log_map(log, cursor, cursor)) {
            streak += 1;
            cursor -= Duration::days(1);
        } else {
            break;
        }
    }

    let mulk_streak = settings.recite_mulk.then(|| {
        let recited: HashMap<NaiveDate, bool> = log
            .mulk_between(today - Duration::days(STREAK_LIMIT), today)
            .into_iter()
            .collect();
        let mut cursor = today;
        // Today counts only once it has been done, so an unfinished day
        // never breaks a run.
        if recited.get(&cursor) != Some(&true) {
            cursor -= Duration::days(1);
        }
        let mut count = 0;
        while recited.get(&cursor) == Some(&true) {
            count += 1;
            cursor -= Duration::days(1);
        }
        count
    });

    Ok(StatsView {
        today: today.to_string(),
        label: format!("{} {}", month_name(month), year),
        year,
        month,
        days,
        streak,
        mulk_streak,
        on_time: (finished > 0).then(|| (prayed * 100) / finished),
        has_next: (year, month) < (today.year(), today.month()),
    })
}

fn log_map(
    log: &PrayerLog,
    from: NaiveDate,
    to: NaiveDate,
) -> HashMap<(NaiveDate, Prayer), LoggedStatus> {
    log.logs_between(from, to)
        .into_iter()
        .map(|(date, prayer, status)| ((date, prayer), status))
        .collect()
}

fn month_name(month: u32) -> &'static str {
    [
        "January", "February", "March", "April", "May", "June", "July", "August", "September",
        "October", "November", "December",
    ]
    .get((month.max(1) - 1) as usize)
    .copied()
    .unwrap_or("")
}

pub fn qaza(log: &PrayerLog) -> QazaView {
    let missed: HashMap<Prayer, i64> = log.missed_counts().into_iter().collect();
    let backlog: HashMap<Prayer, i64> = log.qaza_totals("backlog").into_iter().collect();
    let madeup: HashMap<Prayer, i64> = log.qaza_totals("madeup").into_iter().collect();

    // Kept in prayer order. Ranking by debt reshuffled the list every time
    // a count changed, so the same prayer was never in the same place, and
    // the order that every other screen uses was the one thing this one
    // would not follow.
    let rows: Vec<QazaRow> = Prayer::ALL
        .into_iter()
        .map(|prayer| {
            let owed = missed.get(&prayer).copied().unwrap_or(0)
                + backlog.get(&prayer).copied().unwrap_or(0);
            let made_up = madeup.get(&prayer).copied().unwrap_or(0).min(owed);
            QazaRow {
                prayer,
                label: today::label_for(prayer, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
                remaining: owed - made_up,
                made_up,
                backlog: backlog.get(&prayer).copied().unwrap_or(0),
            }
        })
        .collect();

    QazaView {
        total: rows.iter().map(|row| row.remaining).sum(),
        made_up: rows.iter().map(|row| row.made_up).sum(),
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap()
    }

    #[test]
    fn qaza_counts_missed_plus_backlog_less_make_ups() {
        let log = PrayerLog::in_memory().unwrap();
        log.set(
            NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(),
            Prayer::Fajr,
            LoggedStatus::Missed,
            at(),
        );
        log.set_backlog(Prayer::Fajr, 200, at());
        log.add_madeup(Prayer::Fajr, at());

        let view = qaza(&log);
        let fajr = view.rows.iter().find(|r| r.prayer == Prayer::Fajr).unwrap();
        assert_eq!(fajr.remaining, 200);
        assert_eq!(view.total, 200);
    }

    #[test]
    fn making_up_more_than_owed_settles_at_zero() {
        let log = PrayerLog::in_memory().unwrap();
        log.set_backlog(Prayer::Isha, 1, at());
        log.add_madeup(Prayer::Isha, at());
        log.add_madeup(Prayer::Isha, at());

        let view = qaza(&log);
        let isha = view.rows.iter().find(|r| r.prayer == Prayer::Isha).unwrap();
        assert_eq!(isha.remaining, 0);
    }

    #[test]
    fn rows_stay_in_prayer_order_whatever_is_owed() {
        // Isha owes the most. Ranked by debt it would lead the list; it
        // belongs last, where it is on every other screen.
        let log = PrayerLog::in_memory().unwrap();
        log.set_backlog(Prayer::Fajr, 5, at());
        log.set_backlog(Prayer::Isha, 40, at());

        let view = qaza(&log);
        let order: Vec<Prayer> = view.rows.iter().map(|row| row.prayer).collect();
        assert_eq!(order, Prayer::ALL.to_vec());

    }

    #[test]
    fn progress_counts_what_was_made_up_against_what_was_owed() {
        let log = PrayerLog::in_memory().unwrap();
        log.set_backlog(Prayer::Isha, 30, at());
        for _ in 0..18 {
            log.add_madeup(Prayer::Isha, at());
        }

        let view = qaza(&log);
        let isha = view.rows.iter().find(|row| row.prayer == Prayer::Isha).unwrap();
        assert_eq!((isha.made_up, isha.remaining), (18, 12));
        assert_eq!((view.made_up, view.total), (18, 12));
    }

    #[test]
    fn making_up_more_than_was_owed_counts_only_what_was_owed() {
        // Five made up, then the backlog lowered to three. The bar is drawn
        // against what is owed, so the made-up count stops there too.
        let log = PrayerLog::in_memory().unwrap();
        log.set_backlog(Prayer::Fajr, 5, at());
        for _ in 0..5 {
            log.add_madeup(Prayer::Fajr, at());
        }
        log.set_backlog(Prayer::Fajr, 3, at());

        let fajr = qaza(&log).rows.into_iter().find(|row| row.prayer == Prayer::Fajr).unwrap();
        assert_eq!((fajr.made_up, fajr.remaining), (3, 0));
    }

    #[test]
    fn an_empty_ledger_reports_nothing_owed() {
        let view = qaza(&PrayerLog::in_memory().unwrap());
        assert_eq!(view.total, 0);
        assert!(view.rows.iter().all(|row| row.remaining == 0));
    }

    #[test]
    fn future_prayers_are_upcoming_rather_than_missed() {
        let log = PrayerLog::in_memory().unwrap();
        let settings = Settings::for_tests();
        // Mid-month: later days have not happened yet.
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();
        let view = month_stats(now, &settings, &log, 2026, 6).unwrap();

        let last = view.days.last().unwrap();
        assert!(last.statuses.iter().all(|status| *status == "upcoming"));
        // And an elapsed day with nothing logged reads as unlogged.
        let first = view.days.first().unwrap();
        assert!(first.statuses.iter().all(|status| *status == "unlogged"));
    }

    #[test]
    fn mulk_is_tracked_apart_from_the_prayers() {
        let log = PrayerLog::in_memory().unwrap();
        let settings = Settings::for_tests();
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();

        log.set_mulk(NaiveDate::from_ymd_opt(2026, 6, 10).unwrap(), true, now);
        let view = month_stats(now, &settings, &log, 2026, 6).unwrap();

        let tenth = view.days.iter().find(|d| d.day == 10).unwrap();
        assert_eq!(tenth.mulk, Some("prayed"));
        // The five prayer rows are untouched by it.
        assert_eq!(tenth.statuses.len(), 5);
        assert!(tenth.statuses.iter().all(|s| *s == "unlogged"));
    }

    #[test]
    fn switching_mulk_off_removes_it_from_the_view() {
        let log = PrayerLog::in_memory().unwrap();
        let mut settings = Settings::for_tests();
        settings.recite_mulk = false;
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();

        let view = month_stats(now, &settings, &log, 2026, 6).unwrap();
        assert_eq!(view.mulk_streak, None);
        assert!(view.days.iter().all(|day| day.mulk.is_none()));
    }

    #[test]
    fn the_mulk_streak_counts_back_over_recited_days() {
        let log = PrayerLog::in_memory().unwrap();
        let settings = Settings::for_tests();
        let now = Utc::now();
        let today = today::display_date(now, &settings).unwrap();

        for back in 1..=4 {
            log.set_mulk(today - Duration::days(back), true, now);
        }
        let view = month_stats(now, &settings, &log, today.year(), today.month()).unwrap();
        assert_eq!(view.mulk_streak, Some(4));
    }

    #[test]
    fn a_streak_counts_back_over_fully_prayed_days() {
        let log = PrayerLog::in_memory().unwrap();
        let settings = Settings::for_tests();
        let now = Utc::now();
        let today = today::display_date(now, &settings).unwrap();

        for back in 1..=3 {
            for prayer in Prayer::ALL {
                log.set(today - Duration::days(back), prayer, LoggedStatus::Prayed, now);
            }
        }

        let view = month_stats(now, &settings, &log, today.year(), today.month()).unwrap();
        assert_eq!(view.streak, 3);
    }
}
