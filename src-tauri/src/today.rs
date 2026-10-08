use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc, Weekday};
use salah::Coordinates;
use serde::Serialize;

use crate::engine::{
    build_parameters, compute_day_times, compute_state, window_for, DayTimes, Prayer,
    PrayerCalcError, PrayerState,
};
use crate::settings::Settings;
use crate::store::PrayerLog;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TodayView {
    pub now: String,
    /// The day the view is showing, which before Fajr is still yesterday.
    /// The Mulk row logs against this rather than the wall calendar.
    pub date: String,
    pub timezone: String,
    pub footer: String,
    pub focus: Focus,
    pub rows: Vec<Row>,
    /// None when Surah Mulk tracking is off.
    pub mulk: Option<&'static str>,
}

/// What the countdown ring shows: either the window we're inside, or — in
/// the gap between sunrise and Zuhr — the prayer we're waiting for.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Focus {
    pub prayer: Prayer,
    pub label: &'static str,
    pub arabic: &'static str,
    pub active: bool,
    pub remaining_ms: i64,
    /// Fraction of the window still left, for the ring's arc. Zero while
    /// waiting for a window to open.
    pub window_fraction: f64,
    pub starts_at: String,
    pub ends_at: String,
    pub prayed_at: Option<String>,
}

/// One prayer from a previous day that closed without an answer. The plan
/// calls these Unlogged: never Qaza unless the user says so.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewItem {
    pub date: String,
    pub prayer: Prayer,
    pub label: &'static str,
    pub arabic: &'static str,
    pub day: String,
    pub time: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Row {
    pub key: &'static str,
    pub label: &'static str,
    pub arabic: &'static str,
    pub time: String,
    pub status: &'static str,
    pub prayed_at: Option<String>,
}

pub fn label_for(prayer: Prayer, date: NaiveDate) -> &'static str {
    match prayer {
        Prayer::Fajr => "Fajr",
        Prayer::Zuhr if date.weekday() == Weekday::Fri => "Jumu'ah",
        Prayer::Zuhr => "Zuhr",
        Prayer::Asr => "Asr",
        Prayer::Maghrib => "Maghrib",
        Prayer::Isha => "Isha",
    }
}

pub fn arabic_for(prayer: Prayer, date: NaiveDate) -> &'static str {
    match prayer {
        Prayer::Fajr => "الفجر",
        Prayer::Zuhr if date.weekday() == Weekday::Fri => "الجمعة",
        Prayer::Zuhr => "الظهر",
        Prayer::Asr => "العصر",
        Prayer::Maghrib => "المغرب",
        Prayer::Isha => "العشاء",
    }
}

fn key_for(prayer: Prayer) -> &'static str {
    match prayer {
        Prayer::Fajr => "fajr",
        Prayer::Zuhr => "zuhr",
        Prayer::Asr => "asr",
        Prayer::Maghrib => "maghrib",
        Prayer::Isha => "isha",
    }
}

pub fn day_times_for(date: NaiveDate, settings: &Settings) -> Result<DayTimes, PrayerCalcError> {
    let place = settings
        .location
        .as_ref()
        .ok_or(PrayerCalcError::LocationUnset)?;
    compute_day_times(
        date,
        Coordinates::new(place.latitude, place.longitude),
        build_parameters(
            settings.salah_method(),
            settings.salah_madhab(),
            settings.adjustments,
        ),
    )
}

/// The day whose prayers are relevant right now. Before today's Fajr we are
/// still inside yesterday's Isha window, so the popover keeps showing
/// yesterday's list rather than jumping ahead at midnight.
pub fn display_date(now: DateTime<Utc>, settings: &Settings) -> Result<NaiveDate, PrayerCalcError> {
    let local_today = now.with_timezone(&settings.timezone()).date_naive();
    let today = day_times_for(local_today, settings)?;
    if now < today.fajr {
        return Ok(local_today - Duration::days(1));
    }
    Ok(local_today)
}

pub fn build_view(
    now: DateTime<Utc>,
    settings: &Settings,
    log: &PrayerLog,
) -> Result<TodayView, PrayerCalcError> {
    let date = display_date(now, settings)?;
    let day = day_times_for(date, settings)?;

    let mut rows = Vec::with_capacity(6);
    for prayer in Prayer::ALL {
        let window = window_for(&day, prayer);
        let state = compute_state(now, &window, log.get(date, prayer));
        let (status, prayed_at) = match &state {
            PrayerState::Upcoming => ("upcoming", None),
            PrayerState::Active => ("active", None),
            PrayerState::Prayed { at } => ("prayed", Some(at.to_rfc3339())),
            PrayerState::Unlogged => ("unlogged", None),
            PrayerState::Missed => ("missed", None),
        };
        rows.push(Row {
            key: key_for(prayer),
            label: label_for(prayer, date),
            arabic: arabic_for(prayer, date),
            time: window.start.to_rfc3339(),
            status,
            prayed_at,
        });
        if prayer == Prayer::Fajr {
            rows.push(Row {
                key: "sunrise",
                label: "Sunrise",
                arabic: "الشروق",
                time: day.sunrise.to_rfc3339(),
                status: "none",
                prayed_at: None,
            });
        }
    }

    let focus = build_focus(now, date, &day, log);

    let mulk = settings.recite_mulk.then(
        || match log.mulk_between(date, date).first().map(|(_, done)| *done) {
            Some(true) => "prayed",
            Some(false) => "missed",
            None => "unlogged",
        },
    );

    Ok(TodayView {
        now: now.to_rfc3339(),
        date: date.to_string(),
        timezone: settings.timezone().name().to_string(),
        footer: settings.footer(),
        focus,
        rows,
        mulk,
    })
}

fn build_focus(now: DateTime<Utc>, date: NaiveDate, day: &DayTimes, log: &PrayerLog) -> Focus {
    let active = Prayer::ALL
        .into_iter()
        .find(|prayer| window_for(day, *prayer).contains(now));

    match active {
        Some(prayer) => {
            let window = window_for(day, prayer);
            let remaining = window.end - now;
            let total = window.end - window.start;
            let prayed_at = match compute_state(now, &window, log.get(date, prayer)) {
                PrayerState::Prayed { at } => Some(at.to_rfc3339()),
                _ => None,
            };
            Focus {
                prayer,
                label: label_for(prayer, date),
                arabic: arabic_for(prayer, date),
                active: true,
                remaining_ms: remaining.num_milliseconds(),
                window_fraction: remaining.num_milliseconds() as f64
                    / total.num_milliseconds().max(1) as f64,
                starts_at: window.start.to_rfc3339(),
                ends_at: window.end.to_rfc3339(),
                prayed_at: prayed_at,
            }
        }
        // The only gap between windows is sunrise -> Zuhr.
        None => {
            let (prayer, starts_at) = Prayer::ALL
                .into_iter()
                .map(|prayer| (prayer, window_for(day, prayer).start))
                .find(|(_, start)| *start > now)
                .unwrap_or((Prayer::Fajr, day.next_fajr));
            Focus {
                prayer,
                label: label_for(prayer, date),
                arabic: arabic_for(prayer, date),
                active: false,
                remaining_ms: (starts_at - now).num_milliseconds(),
                window_fraction: 0.0,
                starts_at: starts_at.to_rfc3339(),
                ends_at: starts_at.to_rfc3339(),
                prayed_at: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_has_nothing_to_review() {
        // The review walks back a week and reports every prayer with no log.
        // On an empty database that is all of them — so a brand new user was
        // greeted by "35 prayers unlogged" for days that predate the app.
        let log = PrayerLog::in_memory().unwrap();
        let settings = Settings::for_tests();
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 9, 0, 0).unwrap();

        // `first_run` is today, so there is no history to answer for.
        assert!(pending_review(now, &settings, &log).unwrap().is_empty());
    }

    #[test]
    fn days_inside_a_restored_history_are_still_reviewed() {
        // A restore moves the floor back, so genuinely unlogged days within
        // the history that arrived are still worth asking about.
        let log = PrayerLog::in_memory().unwrap();
        let settings = Settings::for_tests();
        let now = Utc.with_ymd_and_hms(2026, 9, 23, 9, 0, 0).unwrap();

        log.set(
            NaiveDate::from_ymd_opt(2026, 9, 18).unwrap(),
            Prayer::Fajr,
            LoggedStatus::Prayed,
            now - Duration::days(5),
        );

        let items = pending_review(now, &settings, &log).unwrap();
        assert!(!items.is_empty(), "restored days should still be reviewable");
        assert!(
            items.iter().all(|item| item.date >= "2026-09-18".to_string()),
            "nothing before the earliest record should be offered"
        );
    }
    use crate::engine::LoggedStatus;
    use chrono::TimeZone;

    #[test]
    fn before_fajr_still_shows_yesterdays_list() {
        let settings = Settings::for_tests();
        // 01:00 local in Gujranwala (UTC+5) == 20:00 UTC the previous day,
        // well inside the previous day's Isha window.
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 20, 0, 0).unwrap();
        let date = display_date(now, &settings).unwrap();
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 6, 15).unwrap());

        let view = build_view(now, &settings, &PrayerLog::in_memory().unwrap()).unwrap();
        assert_eq!(view.focus.prayer, Prayer::Isha);
        assert!(view.focus.active);
    }

    #[test]
    fn sunrise_to_zuhr_gap_focuses_the_next_prayer() {
        let settings = Settings::for_tests();
        // 09:00 local == 04:00 UTC: after sunrise, before Zuhr.
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 4, 0, 0).unwrap();
        let view = build_view(now, &settings, &PrayerLog::in_memory().unwrap()).unwrap();

        assert_eq!(view.focus.prayer, Prayer::Zuhr);
        assert!(!view.focus.active);
        assert!(view.focus.remaining_ms > 0);
        assert_eq!(view.focus.window_fraction, 0.0);
    }

    #[test]
    fn logging_a_prayer_shows_up_in_the_row_and_focus() {
        let settings = Settings::for_tests();
        // 13:30 local == 08:30 UTC, inside the Zuhr window.
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();
        let log = PrayerLog::in_memory().unwrap();
        let date = display_date(now, &settings).unwrap();
        log.set(date, Prayer::Zuhr, LoggedStatus::Prayed, now);

        let view = build_view(now, &settings, &log).unwrap();
        assert!(view.focus.prayed_at.is_some());
        let zuhr = view.rows.iter().find(|r| r.key == "zuhr").unwrap();
        assert_eq!(zuhr.status, "prayed");
    }

    #[test]
    fn the_mulk_row_reflects_the_day_being_shown() {
        let settings = Settings::for_tests();
        let log = PrayerLog::in_memory().unwrap();
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();
        let date = display_date(now, &settings).unwrap();

        assert_eq!(build_view(now, &settings, &log).unwrap().mulk, Some("unlogged"));

        log.set_mulk(date, true, now);
        let view = build_view(now, &settings, &log).unwrap();
        assert_eq!(view.mulk, Some("prayed"));
        assert_eq!(view.date, date.to_string());
    }

    #[test]
    fn switching_mulk_off_drops_it_from_today() {
        let mut settings = Settings::for_tests();
        settings.recite_mulk = false;
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();
        let view = build_view(now, &settings, &PrayerLog::in_memory().unwrap()).unwrap();
        assert_eq!(view.mulk, None);
    }

    #[test]
    fn friday_zuhr_is_labelled_jumuah() {
        let friday = NaiveDate::from_ymd_opt(2026, 6, 19).unwrap();
        assert_eq!(friday.weekday(), Weekday::Fri);
        assert_eq!(label_for(Prayer::Zuhr, friday), "Jumu'ah");
        assert_eq!(label_for(Prayer::Zuhr, friday - Duration::days(1)), "Zuhr");
    }

    #[test]
    fn rows_cover_all_five_prayers_plus_sunrise_in_order() {
        let settings = Settings::for_tests();
        let now = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();
        let view = build_view(now, &settings, &PrayerLog::in_memory().unwrap()).unwrap();

        let keys: Vec<&str> = view.rows.iter().map(|r| r.key).collect();
        assert_eq!(
            keys,
            vec!["fajr", "sunrise", "zuhr", "asr", "maghrib", "isha"]
        );
    }
}

/// How far back the morning review looks. Bounded so a laptop left shut for
/// a month does not open onto an unusable wall of rows.
const REVIEW_DAYS: i64 = 7;

/// Prayers from earlier days whose window closed with nothing logged.
/// Returns nothing once the review has already been shown today, which is
/// what makes it a once-a-day card rather than a permanent nag.
pub fn pending_review(
    now: DateTime<Utc>,
    settings: &Settings,
    log: &PrayerLog,
) -> Result<Vec<ReviewItem>, PrayerCalcError> {
    let today = display_date(now, settings)?;
    if log.last_review() == Some(today) {
        return Ok(Vec::new());
    }

    // Nothing before this database existed, and nothing at all if it has no
    // history: a first run has no prayers to have missed.
    let Some(floor) = log.earliest_known_day() else {
        return Ok(Vec::new());
    };

    let mut items = Vec::new();
    for back in (1..=REVIEW_DAYS).rev() {
        let date = today - Duration::days(back);
        if date < floor {
            continue;
        }
        let Ok(day) = day_times_for(date, settings) else {
            continue;
        };

        for prayer in Prayer::ALL {
            let window = window_for(&day, prayer);
            if window.end > now || log.get(date, prayer).is_some() {
                continue;
            }
            items.push(ReviewItem {
                date: date.to_string(),
                prayer,
                label: label_for(prayer, date),
                arabic: arabic_for(prayer, date),
                day: date.format("%a").to_string(),
                time: window.start.to_rfc3339(),
            });
        }
    }
    Ok(items)
}
