use std::path::Path;
use std::sync::Mutex;

use chrono::{DateTime, NaiveDate, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension};

use crate::engine::{LoggedStatus, Prayer};

/// Every prayer the user has logged, on disk.
///
/// `updated_at` and `device_id` are written from the start even though
/// nothing reads them yet: the plan adds sync later, and backfilling them
/// onto existing rows would mean a migration with no way to recover the
/// real values.
pub struct PrayerLog {
    connection: Mutex<Connection>,
    device_id: String,
}

fn status_name(status: LoggedStatus) -> &'static str {
    match status {
        LoggedStatus::Prayed => "prayed",
        LoggedStatus::Missed => "missed",
    }
}

fn status_from(name: &str) -> Option<LoggedStatus> {
    match name {
        "prayed" => Some(LoggedStatus::Prayed),
        "missed" => Some(LoggedStatus::Missed),
        _ => None,
    }
}

/// The one place a timestamp is formatted.
///
/// Sync orders records by comparing these as text, which is only
/// chronological while every one is canonical: UTC, `Z` suffix, whole
/// seconds. A stray offset or fractional precision would silently break
/// conflict resolution rather than fail loudly, so all writes go through here.
pub fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn prayer_name(prayer: Prayer) -> &'static str {
    match prayer {
        Prayer::Fajr => "fajr",
        Prayer::Zuhr => "zuhr",
        Prayer::Asr => "asr",
        Prayer::Maghrib => "maghrib",
        Prayer::Isha => "isha",
    }
}

pub fn prayer_from(name: &str) -> Option<Prayer> {
    match name {
        "fajr" => Some(Prayer::Fajr),
        "zuhr" => Some(Prayer::Zuhr),
        "asr" => Some(Prayer::Asr),
        "maghrib" => Some(Prayer::Maghrib),
        "isha" => Some(Prayer::Isha),
        _ => None,
    }
}

impl PrayerLog {
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> rusqlite::Result<Self> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS prayer_logs (
                date       TEXT NOT NULL,
                prayer     TEXT NOT NULL,
                status     TEXT NOT NULL,
                logged_at  TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                device_id  TEXT NOT NULL,
                PRIMARY KEY (date, prayer)
            );
            CREATE TABLE IF NOT EXISTS qaza_entries (
                id         TEXT PRIMARY KEY,
                prayer     TEXT NOT NULL,
                kind       TEXT NOT NULL,
                count      INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                device_id  TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS mulk_logs (
                date       TEXT PRIMARY KEY,
                status     TEXT NOT NULL,
                logged_at  TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                device_id  TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS reminder_log (
                date    TEXT NOT NULL,
                prayer  TEXT NOT NULL,
                fired_at TEXT NOT NULL,
                PRIMARY KEY (date, prayer)
            );
            CREATE TABLE IF NOT EXISTS meta (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );",
        )?;

        let existing: Option<String> = connection
            .query_row("SELECT value FROM meta WHERE key = 'device_id'", [], |row| {
                row.get(0)
            })
            .optional()?;

        let device_id = match existing {
            Some(id) => id,
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                connection.execute(
                    "INSERT INTO meta (key, value) VALUES ('device_id', ?1)",
                    [&id],
                )?;
                id
            }
        };

        let log = Self {
            connection: Mutex::new(connection),
            device_id,
        };
        // Recorded once, on creation. It is what tells a fresh install that
        // it has no past to answer for.
        if log.load_json::<String>("first_run").is_none() {
            log.save_json("first_run", &Utc::now().date_naive().to_string());
        }
        Ok(log)
    }

    /// `logged_at` moves with the status: it records when *this* answer was
    /// given, so correcting a prayer from history does not leave the old
    /// decision's timestamp attached to the new one.
    pub fn set(&self, date: NaiveDate, prayer: Prayer, status: LoggedStatus, at: DateTime<Utc>) {
        let stamp = stamp(at);
        let connection = self.connection.lock().unwrap();
        let _ = connection.execute(
            "INSERT INTO prayer_logs (date, prayer, status, logged_at, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)
             ON CONFLICT(date, prayer) DO UPDATE SET
                status = excluded.status,
                logged_at = excluded.logged_at,
                updated_at = excluded.updated_at,
                device_id = excluded.device_id",
            rusqlite::params![
                date.to_string(),
                prayer_name(prayer),
                status_name(status),
                stamp,
                self.device_id,
            ],
        );
    }

    pub fn get(&self, date: NaiveDate, prayer: Prayer) -> Option<(LoggedStatus, DateTime<Utc>)> {
        let connection = self.connection.lock().unwrap();
        let row: Option<(String, String)> = connection
            .query_row(
                "SELECT status, logged_at FROM prayer_logs WHERE date = ?1 AND prayer = ?2",
                rusqlite::params![date.to_string(), prayer_name(prayer)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .ok()
            .flatten();

        let (status, logged_at) = row?;
        Some((
            status_from(&status)?,
            DateTime::parse_from_rfc3339(&logged_at)
                .ok()?
                .with_timezone(&Utc),
        ))
    }

    /// Every prayer logged between two dates, for the month grid.
    pub fn logs_between(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Vec<(NaiveDate, Prayer, LoggedStatus)> {
        let connection = self.connection.lock().unwrap();
        let Ok(mut statement) = connection.prepare(
            "SELECT date, prayer, status FROM prayer_logs WHERE date >= ?1 AND date <= ?2",
        ) else {
            return Vec::new();
        };

        let rows = statement.query_map(
            rusqlite::params![from.to_string(), to.to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        );

        let Ok(rows) = rows else {
            return Vec::new();
        };
        rows.filter_map(|row| {
            let (date, prayer, status) = row.ok()?;
            Some((
                date.parse().ok()?,
                prayer_from(&prayer)?,
                status_from(&status)?,
            ))
        })
        .collect()
    }

    /// How many prayers the user has marked Missed, per prayer. Only these
    /// count as Qaza — an unanswered prayer stays Unlogged, never Qaza.
    pub fn missed_counts(&self) -> Vec<(Prayer, i64)> {
        let connection = self.connection.lock().unwrap();
        let Ok(mut statement) = connection
            .prepare("SELECT prayer, COUNT(*) FROM prayer_logs WHERE status = 'missed' GROUP BY prayer")
        else {
            return Vec::new();
        };
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        });
        let Ok(rows) = rows else {
            return Vec::new();
        };
        rows.filter_map(|row| {
            let (prayer, count) = row.ok()?;
            Some((prayer_from(&prayer)?, count))
        })
        .collect()
    }

    /// Totals per prayer for one kind of Qaza entry: the old backlog, or the
    /// make-ups logged since.
    pub fn qaza_totals(&self, kind: &str) -> Vec<(Prayer, i64)> {
        let connection = self.connection.lock().unwrap();
        let Ok(mut statement) = connection.prepare(
            "SELECT prayer, COALESCE(SUM(count), 0) FROM qaza_entries WHERE kind = ?1 GROUP BY prayer",
        ) else {
            return Vec::new();
        };
        let rows = statement.query_map([kind], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        });
        let Ok(rows) = rows else {
            return Vec::new();
        };
        rows.filter_map(|row| {
            let (prayer, count) = row.ok()?;
            Some((prayer_from(&prayer)?, count))
        })
        .collect()
    }

    /// Each make-up is its own row rather than a running total, so the
    /// remaining count is always recalculated and cannot drift once two
    /// devices are syncing.
    pub fn add_madeup(&self, prayer: Prayer, at: DateTime<Utc>) {
        let stamp = stamp(at);
        let connection = self.connection.lock().unwrap();
        let _ = connection.execute(
            "INSERT INTO qaza_entries (id, prayer, kind, count, created_at, updated_at, device_id)
             VALUES (?1, ?2, 'madeup', 1, ?3, ?3, ?4)",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                prayer_name(prayer),
                stamp,
                self.device_id,
            ],
        );
    }

    /// The backlog is a single figure the user states, so it is one row per
    /// prayer that gets replaced rather than appended to.
    pub fn set_backlog(&self, prayer: Prayer, count: i64, at: DateTime<Utc>) {
        let stamp = stamp(at);
        let connection = self.connection.lock().unwrap();
        let _ = connection.execute(
            "INSERT INTO qaza_entries (id, prayer, kind, count, created_at, updated_at, device_id)
             VALUES (?1, ?2, 'backlog', ?3, ?4, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                count = excluded.count,
                updated_at = excluded.updated_at,
                device_id = excluded.device_id",
            rusqlite::params![
                format!("backlog-{}", prayer_name(prayer)),
                prayer_name(prayer),
                count.max(0),
                stamp,
                self.device_id,
            ],
        );
    }

    /// Surah Mulk lives apart from `prayer_logs` on purpose: a missed
    /// recitation is not a missed prayer, and sharing the table would feed
    /// it straight into the Qaza count.
    pub fn set_mulk(&self, date: NaiveDate, recited: bool, at: DateTime<Utc>) {
        let stamp = stamp(at);
        let connection = self.connection.lock().unwrap();
        let _ = connection.execute(
            "INSERT INTO mulk_logs (date, status, logged_at, updated_at, device_id)
             VALUES (?1, ?2, ?3, ?3, ?4)
             ON CONFLICT(date) DO UPDATE SET
                status = excluded.status,
                logged_at = excluded.logged_at,
                updated_at = excluded.updated_at,
                device_id = excluded.device_id",
            rusqlite::params![
                date.to_string(),
                if recited { "recited" } else { "missed" },
                stamp,
                self.device_id,
            ],
        );
    }

    /// Every Mulk entry in a range, as (date, recited).
    pub fn mulk_between(&self, from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, bool)> {
        let connection = self.connection.lock().unwrap();
        let Ok(mut statement) = connection
            .prepare("SELECT date, status FROM mulk_logs WHERE date >= ?1 AND date <= ?2")
        else {
            return Vec::new();
        };
        let rows = statement.query_map(
            rusqlite::params![from.to_string(), to.to_string()],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        );
        let Ok(rows) = rows else {
            return Vec::new();
        };
        rows.filter_map(|row| {
            let (date, status) = row.ok()?;
            Some((date.parse().ok()?, status == "recited"))
        })
        .collect()
    }

    /// When the last reminder for a prayer was fired.
    ///
    /// This is operational bookkeeping, not prayer state. Prayer state stays
    /// derived — it can always be recalculated from what the user did and the
    /// time. Whether a reminder already went out cannot be derived from
    /// anything, so it is the one thing the scheduler must remember, and
    /// keeping it in memory meant a restart re-fired the reminder.
    pub fn last_reminder(&self, date: NaiveDate, prayer: Prayer) -> Option<DateTime<Utc>> {
        let connection = self.connection.lock().unwrap();
        let value: Option<String> = connection
            .query_row(
                "SELECT fired_at FROM reminder_log WHERE date = ?1 AND prayer = ?2",
                rusqlite::params![date.to_string(), prayer_name(prayer)],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();

        value.and_then(|raw| {
            DateTime::parse_from_rfc3339(&raw)
                .ok()
                .map(|parsed| parsed.with_timezone(&Utc))
        })
    }

    pub fn record_reminder(&self, date: NaiveDate, prayer: Prayer, at: DateTime<Utc>) {
        let connection = self.connection.lock().unwrap();
        let _ = connection.execute(
            "INSERT INTO reminder_log (date, prayer, fired_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(date, prayer) DO UPDATE SET fired_at = excluded.fired_at",
            rusqlite::params![date.to_string(), prayer_name(prayer), stamp(at)],
        );
    }

    /// This machine's identity, used to break ties when two devices wrote
    /// the same record in the same second.

    /// `meta` holds whatever does not deserve a table of its own, as JSON
    /// under a named key. Settings are stored this way, split by scope:
    /// see `settings::load`.
    pub fn load_json<T: serde::de::DeserializeOwned>(&self, key: &str) -> Option<T> {
        let connection = self.connection.lock().unwrap();
        let raw: Option<String> = connection
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .ok()
            .flatten();
        serde_json::from_str(&raw?).ok()
    }

    pub fn save_json<T: serde::Serialize>(&self, key: &str, value: &T) {
        let Ok(text) = serde_json::to_string(value) else {
            return;
        };
        let connection = self.connection.lock().unwrap();
        let _ = connection.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, text],
        );
    }

    /// The earliest day this database knows anything about.
    ///
    /// Days before it were not missed — they were before the app existed
    /// here. Without this floor a fresh install opens by announcing that the
    /// user has 35 unlogged prayers, which is both untrue and a miserable
    /// first impression. A restore moves the floor back, so genuinely
    /// unlogged days inside the restored history are still offered.
    pub fn earliest_known_day(&self) -> Option<NaiveDate> {
        let connection = self.connection.lock().unwrap();
        let raw: Option<String> = connection
            .query_row(
                // `first_run` is stored as JSON, so it arrives wrapped in
                // quotes — and `\"` sorts before a digit, which made MIN
                // pick it over every real date. Strip before comparing.
                "SELECT MIN(day) FROM (
                    SELECT MIN(date) AS day FROM prayer_logs
                    UNION ALL SELECT MIN(date) FROM mulk_logs
                    UNION ALL SELECT trim(value, '\"') FROM meta WHERE key = 'first_run'
                 ) WHERE day IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        raw.and_then(|value| value.trim_matches('"').parse().ok())
    }

    /// The last date the morning review was shown, so it appears once a day.
    pub fn last_review(&self) -> Option<NaiveDate> {
        let connection = self.connection.lock().unwrap();
        let value: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_review'",
                [],
                |row| row.get(0),
            )
            .optional()
            .ok()
            .flatten();
        value.and_then(|raw| raw.parse().ok())
    }

    pub fn mark_reviewed(&self, date: NaiveDate) {
        let connection = self.connection.lock().unwrap();
        let _ = connection.execute(
            "INSERT INTO meta (key, value) VALUES ('last_review', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [date.to_string()],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 6, 15).unwrap()
    }

    #[test]
    fn every_timestamp_is_canonical() {
        // Text ordering equals chronological ordering only while this holds.
        let at = Utc.with_ymd_and_hms(2026, 9, 1, 6, 0, 0).unwrap();
        assert_eq!(stamp(at), "2026-09-01T06:00:00Z");

        let log = PrayerLog::in_memory().unwrap();
        log.set(date(), Prayer::Fajr, LoggedStatus::Prayed, at);
        log.set_mulk(date(), true, at);
        log.add_madeup(Prayer::Fajr, at);
        log.set_backlog(Prayer::Isha, 3, at);

        let connection = log.connection.lock().unwrap();
        for query in [
            "SELECT updated_at FROM prayer_logs",
            "SELECT updated_at FROM mulk_logs",
            "SELECT updated_at FROM qaza_entries",
        ] {
            let mut statement = connection.prepare(query).unwrap();
            let rows: Vec<String> = statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|value| value.unwrap())
                .collect();
            assert!(!rows.is_empty(), "{query} wrote nothing");
            for value in rows {
                assert!(
                    value.ends_with('Z') && value.len() == 20,
                    "{query} produced a non-canonical stamp: {value}"
                );
            }
        }
    }

    #[test]
    fn a_logged_prayer_reads_back() {
        let log = PrayerLog::in_memory().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();

        assert_eq!(log.get(date(), Prayer::Zuhr), None);
        log.set(date(), Prayer::Zuhr, LoggedStatus::Prayed, at);
        assert_eq!(log.get(date(), Prayer::Zuhr), Some((LoggedStatus::Prayed, at)));
    }

    #[test]
    fn logging_the_same_prayer_twice_replaces_it() {
        let log = PrayerLog::in_memory().unwrap();
        let first = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();
        let second = Utc.with_ymd_and_hms(2026, 6, 15, 9, 0, 0).unwrap();

        log.set(date(), Prayer::Asr, LoggedStatus::Prayed, first);
        log.set(date(), Prayer::Asr, LoggedStatus::Missed, second);

        assert_eq!(
            log.get(date(), Prayer::Asr),
            Some((LoggedStatus::Missed, second))
        );
    }

    #[test]
    fn logs_survive_reopening_the_file() {
        let directory = std::env::temp_dir().join(format!("sakina-{}", uuid::Uuid::new_v4()));
        let path = directory.join("sakina.db");
        let at = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();

        {
            let log = PrayerLog::open(&path).unwrap();
            log.set(date(), Prayer::Fajr, LoggedStatus::Prayed, at);
        }

        let reopened = PrayerLog::open(&path).unwrap();
        assert_eq!(
            reopened.get(date(), Prayer::Fajr),
            Some((LoggedStatus::Prayed, at))
        );

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_device_id_is_stable_across_opens() {
        let directory = std::env::temp_dir().join(format!("sakina-{}", uuid::Uuid::new_v4()));
        let path = directory.join("sakina.db");

        let first = PrayerLog::open(&path).unwrap().device_id.clone();
        let second = PrayerLog::open(&path).unwrap().device_id.clone();

        assert_eq!(first, second);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn only_missed_prayers_count_toward_qaza() {
        let log = PrayerLog::in_memory().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();

        log.set(date(), Prayer::Fajr, LoggedStatus::Missed, at);
        log.set(
            date() + chrono::Duration::days(1),
            Prayer::Fajr,
            LoggedStatus::Missed,
            at,
        );
        log.set(date(), Prayer::Zuhr, LoggedStatus::Prayed, at);

        let counts = log.missed_counts();
        assert_eq!(
            counts.iter().find(|(p, _)| *p == Prayer::Fajr).map(|(_, n)| *n),
            Some(2)
        );
        // A prayed prayer is not Qaza, and neither is an unanswered one.
        assert!(counts.iter().all(|(p, _)| *p != Prayer::Zuhr));
        assert!(counts.iter().all(|(p, _)| *p != Prayer::Asr));
    }

    #[test]
    fn make_ups_accumulate_but_the_backlog_is_replaced() {
        let log = PrayerLog::in_memory().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();

        log.add_madeup(Prayer::Fajr, at);
        log.add_madeup(Prayer::Fajr, at);
        assert_eq!(log.qaza_totals("madeup"), vec![(Prayer::Fajr, 2)]);

        log.set_backlog(Prayer::Fajr, 200, at);
        log.set_backlog(Prayer::Fajr, 150, at);
        assert_eq!(log.qaza_totals("backlog"), vec![(Prayer::Fajr, 150)]);
    }

    #[test]
    fn logs_between_covers_only_the_range() {
        let log = PrayerLog::in_memory().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();
        let inside = date();
        let outside = date() - chrono::Duration::days(10);

        log.set(inside, Prayer::Asr, LoggedStatus::Prayed, at);
        log.set(outside, Prayer::Asr, LoggedStatus::Prayed, at);

        let rows = log.logs_between(date() - chrono::Duration::days(3), date());
        assert_eq!(rows, vec![(inside, Prayer::Asr, LoggedStatus::Prayed)]);
    }

    #[test]
    fn a_missed_mulk_never_reaches_the_qaza_count() {
        let log = PrayerLog::in_memory().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();

        log.set_mulk(date(), false, at);
        log.set_mulk(date() - chrono::Duration::days(1), true, at);

        // Qaza is built from prayer_logs alone.
        assert!(log.missed_counts().is_empty());

        let entries = log.mulk_between(date() - chrono::Duration::days(1), date());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries.iter().find(|(d, _)| *d == date()).unwrap().1, false);
    }

    #[test]
    fn logging_mulk_twice_replaces_it() {
        let log = PrayerLog::in_memory().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 6, 15, 8, 30, 0).unwrap();

        log.set_mulk(date(), true, at);
        log.set_mulk(date(), false, at);
        assert_eq!(log.mulk_between(date(), date()), vec![(date(), false)]);
    }

    #[test]
    fn the_review_date_round_trips() {
        let log = PrayerLog::in_memory().unwrap();
        assert_eq!(log.last_review(), None);
        log.mark_reviewed(date());
        assert_eq!(log.last_review(), Some(date()));
    }
}
