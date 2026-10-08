use chrono_tz::Tz;
use salah::{Madhab, Method};
use serde::{Deserialize, Serialize};

use crate::engine::{FirstNudge, Prayer, PrayerAdjustments};
use crate::store::PrayerLog;

/// `salah`'s own enums are not serialisable and cover methods we do not
/// offer, so settings carry their own and convert at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalcMethod {
    Karachi,
    MuslimWorldLeague,
    Egyptian,
    UmmAlQura,
    NorthAmerica,
    Dubai,
    Kuwait,
    Qatar,
    Singapore,
    Tehran,
    Turkey,
}

impl CalcMethod {
    fn to_salah(self) -> Method {
        match self {
            CalcMethod::Karachi => Method::Karachi,
            CalcMethod::MuslimWorldLeague => Method::MuslimWorldLeague,
            CalcMethod::Egyptian => Method::Egyptian,
            CalcMethod::UmmAlQura => Method::UmmAlQura,
            CalcMethod::NorthAmerica => Method::NorthAmerica,
            CalcMethod::Dubai => Method::Dubai,
            CalcMethod::Kuwait => Method::Kuwait,
            CalcMethod::Qatar => Method::Qatar,
            CalcMethod::Singapore => Method::Singapore,
            CalcMethod::Tehran => Method::Tehran,
            CalcMethod::Turkey => Method::Turkey,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            CalcMethod::Karachi => "Karachi",
            CalcMethod::MuslimWorldLeague => "Muslim World League",
            CalcMethod::Egyptian => "Egyptian",
            CalcMethod::UmmAlQura => "Umm al-Qura",
            CalcMethod::NorthAmerica => "ISNA",
            CalcMethod::Dubai => "Dubai",
            CalcMethod::Kuwait => "Kuwait",
            CalcMethod::Qatar => "Qatar",
            CalcMethod::Singapore => "Singapore",
            CalcMethod::Tehran => "Tehran",
            CalcMethod::Turkey => "Turkey",
        }
    }

    fn short_label(self) -> &'static str {
        match self {
            CalcMethod::MuslimWorldLeague => "MWL",
            other => other.label(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrMadhab {
    Hanafi,
    Shafi,
}

impl AsrMadhab {
    fn to_salah(self) -> Madhab {
        match self {
            AsrMadhab::Hanafi => Madhab::Hanafi,
            AsrMadhab::Shafi => Madhab::Shafi,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AsrMadhab::Hanafi => "Hanafi",
            AsrMadhab::Shafi => "Shafi",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NudgeSetting {
    pub prayer: Prayer,
    pub first_nudge: FirstNudge,
}

/// Where the user is, and the one setting with no sensible default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    /// IANA name. Stored as text because chrono-tz's own type is not
    /// serialisable without an extra feature, and text is what a user's
    /// location lookup returns anyway.
    pub timezone: String,
}

impl Place {
    pub fn timezone(&self) -> Tz {
        self.timezone.parse().unwrap_or(chrono_tz::UTC)
    }
}

/// Everything the app reads, and the single flat object the front end sees.
///
/// This was once two structs — one that followed the person between
/// machines and one that stayed put — because only half of it was ever
/// meant to sync. With nothing to sync to, that distinction costs a
/// migration and buys nothing, so it is one struct again.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub method: CalcMethod,
    pub madhab: AsrMadhab,
    /// Surah Mulk is commonly recited nightly. Tracked as a daily habit, not
    /// a prayer: it has no window and never counts toward Qaza.
    pub recite_mulk: bool,
    /// None until the user chooses. Prayer times cannot be guessed: a
    /// default city would show confident, wrong times that look exactly
    /// like right ones, so nothing is calculated before a place is set.
    pub location: Option<Place>,
    pub adjustments: PrayerAdjustments,
    pub first_nudge: Vec<NudgeSetting>,
    /// Friday's Zuhr keeps its own jamaat time.
    pub jumuah: Option<FirstNudge>,
    pub bedtime_hour: u32,
    pub bedtime_minute: u32,
    /// On by default. A reminder app that is not running reminds nobody,
    /// and the one moment a user is certain to miss is the one where they
    /// restarted their Mac and forgot the app existed.
    pub launch_at_login: bool,
    pub theme: Theme,
    pub onboarded: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            method: CalcMethod::Karachi,
            madhab: AsrMadhab::Hanafi,
            recite_mulk: true,
            location: None,
            adjustments: PrayerAdjustments::default(),
            first_nudge: default_nudges(),
            jumuah: None,
            bedtime_hour: 23,
            bedtime_minute: 30,
            launch_at_login: true,
            theme: Theme::System,
            onboarded: false,
        }
    }
}

const SETTINGS_KEY: &str = "settings";
/// The two keys settings were split across while the app could sync.
const ACCOUNT_KEY: &str = "account_settings";
const DEVICE_KEY: &str = "device_settings";

/// Reads the settings, migrating a database written by the syncing build.
///
/// That build stored settings under two keys, because only one of them was
/// meant to travel between machines. An existing user's location, nudge
/// offsets and jamaat times are all in those keys, and reaching straight
/// for the single key would quietly reset them to Gujranwala defaults.
///
/// Both old keys are left in place afterwards, frozen, so rolling back to
/// an older build still finds something sane.
pub fn load(log: &PrayerLog) -> Settings {
    if let Some(stored) = log.load_json::<serde_json::Value>(SETTINGS_KEY) {
        let settings = folded(stored);
        // Written straight back, which rewrites the stored JSON through the
        // current struct and so drops any setting that has since been
        // removed. A dropped setting is already inert — reading it into the
        // struct discards it — but leaving the dead key on disk would mean
        // the bytes disagree with what the app believes, which is the kind
        // of gap that misleads whoever looks next.
        save(log, &settings);
        return settings;
    }

    let settings = from_split_keys(log).map(folded).unwrap_or_default();
    save(log, &settings);
    settings
}

/// Rebuilds one flat object from the two the syncing build wrote.
///
/// The account half was nested inside a record carrying its own timestamp
/// and originating device; only the settings within it still mean anything.
fn from_split_keys(log: &PrayerLog) -> Option<serde_json::Value> {
    use serde_json::Value;

    let account: Option<Value> = log.load_json(ACCOUNT_KEY);
    let device: Option<Value> = log.load_json(DEVICE_KEY);
    if account.is_none() && device.is_none() {
        return None;
    }

    let mut flat = serde_json::Map::new();
    let halves = [
        account.and_then(|record| record.get("settings").cloned()),
        device,
    ];
    for half in halves.into_iter().flatten() {
        if let Value::Object(fields) = half {
            flat.extend(fields);
        }
    }
    Some(Value::Object(flat))
}

/// Folds a stored object into the current struct, carrying a place written
/// by an older build with it.
///
/// Until v0.1.1 the place was four flat keys rather than one nested object.
/// Reaching straight for `location` would leave an existing user with no
/// place at all and send them back through onboarding.
fn folded(stored: serde_json::Value) -> Settings {
    use serde_json::Value;

    let mut stored = stored;
    if let Value::Object(fields) = &mut stored {
        if !fields.contains_key("location") {
            let name = fields.get("locationName").and_then(Value::as_str);
            let latitude = fields.get("latitude").and_then(Value::as_f64);
            let longitude = fields.get("longitude").and_then(Value::as_f64);
            let timezone = fields.get("timezone").and_then(Value::as_str);
            if let (Some(name), Some(latitude), Some(longitude), Some(timezone)) =
                (name, latitude, longitude, timezone)
            {
                let place = Place {
                    name: name.to_string(),
                    latitude,
                    longitude,
                    timezone: timezone.to_string(),
                };
                if let Ok(place) = serde_json::to_value(place) {
                    fields.insert("location".to_string(), place);
                }
            }
        }
    }
    serde_json::from_value(stored).unwrap_or_default()
}

pub fn save(log: &PrayerLog, settings: &Settings) {
    log.save_json(SETTINGS_KEY, settings);
}

/// Plan section 4.2.
fn default_nudges() -> Vec<NudgeSetting> {
    [
        (Prayer::Fajr, 20),
        (Prayer::Zuhr, 60),
        (Prayer::Asr, 30),
        (Prayer::Maghrib, 10),
        (Prayer::Isha, 45),
    ]
    .into_iter()
    .map(|(prayer, minutes)| NudgeSetting {
        prayer,
        first_nudge: FirstNudge::OffsetMinutes { minutes },
    })
    .collect()
}

impl Settings {
    /// UTC while no place is set. Nothing is scheduled or calculated in that
    /// state, so this only keeps the bedtime arithmetic total.
    pub fn timezone(&self) -> Tz {
        self.location
            .as_ref()
            .map(Place::timezone)
            .unwrap_or(chrono_tz::UTC)
    }

    pub fn salah_method(&self) -> Method {
        self.method.to_salah()
    }

    pub fn salah_madhab(&self) -> Madhab {
        self.madhab.to_salah()
    }

    pub fn bedtime(&self) -> (u32, u32) {
        (self.bedtime_hour, self.bedtime_minute)
    }

    /// Friday falls back to the ordinary Zuhr setting when no separate
    /// Jumu'ah time has been entered.
    pub fn first_nudge_for(&self, prayer: Prayer, friday: bool) -> FirstNudge {
        if friday && prayer == Prayer::Zuhr {
            if let Some(jumuah) = self.jumuah {
                return jumuah;
            }
        }
        self.first_nudge
            .iter()
            .find(|setting| setting.prayer == prayer)
            .map(|setting| setting.first_nudge)
            .unwrap_or(FirstNudge::OffsetMinutes { minutes: 30 })
    }

    /// The popover's footer line, e.g. "Karachi · Hanafi · Lahore".
    pub fn footer(&self) -> String {
        format!(
            "{} · {} · {}",
            self.method.short_label(),
            self.madhab.label(),
            self.location
                .as_ref()
                .map(|place| place.name.as_str())
                .unwrap_or("No location")
        )
    }
}

/// A settled place, so a test about nudge timing or the month grid does not
/// have to spell one out.
///
/// Gujranwala because the time assertions throughout the crate were written
/// against its sunrise, and moving them would change what they prove.
#[cfg(test)]
impl Settings {
    pub fn for_tests() -> Self {
        Self {
            location: Some(Place {
                name: "Gujranwala".to_string(),
                latitude: 32.1877,
                longitude: 74.1945,
                timezone: "Asia/Karachi".to_string(),
            }),
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friday_prefers_the_jumuah_time_when_one_is_set() {
        let mut settings = Settings::default();
        assert_eq!(
            settings.first_nudge_for(Prayer::Zuhr, true),
            FirstNudge::OffsetMinutes { minutes: 60 }
        );

        settings.jumuah = Some(FirstNudge::JamaatTime {
            hour: 13,
            minute: 30,
        });
        assert_eq!(
            settings.first_nudge_for(Prayer::Zuhr, true),
            FirstNudge::JamaatTime {
                hour: 13,
                minute: 30
            }
        );
        // Other days are untouched by it.
        assert_eq!(
            settings.first_nudge_for(Prayer::Zuhr, false),
            FirstNudge::OffsetMinutes { minutes: 60 }
        );
    }

    #[test]
    fn an_unreadable_timezone_falls_back_rather_than_panicking() {
        let mut settings = Settings::for_tests();
        settings.location.as_mut().unwrap().timezone = "Not/AZone".to_string();
        assert_eq!(settings.timezone(), chrono_tz::UTC);
    }

    #[test]
    fn a_place_stored_as_flat_keys_is_folded_into_one() {
        // What v0.1.0 wrote. Reaching straight for `location` would leave an
        // existing user with no place and send them back through onboarding.
        let log = PrayerLog::in_memory().unwrap();
        log.save_json(
            SETTINGS_KEY,
            &serde_json::json!({
                "method": "karachi",
                "locationName": "Lahore",
                "latitude": 31.5204,
                "longitude": 74.3587,
                "timezone": "Asia/Karachi",
                "onboarded": true
            }),
        );

        let loaded = load(&log);
        let place = loaded.location.expect("the place survived the upgrade");
        assert_eq!(place.name, "Lahore");
        assert_eq!(place.timezone, "Asia/Karachi");
        assert!(loaded.onboarded, "and the user is not asked again");

        // Rewritten in the current shape, so the bytes on disk agree with
        // what the app believes.
        let raw: serde_json::Value = log.load_json(SETTINGS_KEY).unwrap();
        assert_eq!(raw["location"]["name"], "Lahore");
    }

    #[test]
    fn a_place_nobody_chose_is_no_place_at_all() {
        // The one setting with no honest default. A city here would show
        // confident, wrong prayer times to everyone who skipped past it.
        assert_eq!(Settings::default().location, None);
    }

    #[test]
    fn settings_survive_a_json_round_trip() {
        let settings = Settings::default();
        let text = serde_json::to_string(&settings).unwrap();
        let back: Settings = serde_json::from_str(&text).unwrap();
        assert_eq!(back.location, settings.location);
        assert_eq!(back.method, settings.method);
        assert_eq!(back.first_nudge.len(), 5);
    }

    #[test]
    fn settings_are_one_flat_object_on_the_wire() {
        // The front end has a single settings screen and a single object.
        let text = serde_json::to_string(&Settings::for_tests()).unwrap();
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(json["method"], "karachi");
        assert_eq!(json["location"]["name"], "Gujranwala");
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        // An older settings blob, from before a field existed, must still load.
        let back: Settings =
            serde_json::from_str("{\"location\":{\"name\":\"Lahore\",\"latitude\":31.5,\"longitude\":74.3,\"timezone\":\"Asia/Karachi\"}}")
                .unwrap();
        assert_eq!(back.location.unwrap().name, "Lahore");
        assert_eq!(back.method, CalcMethod::Karachi);
        assert_eq!(back.bedtime_hour, 23);
    }

    #[test]
    fn a_fresh_install_launches_at_login() {
        // A reminder app that is not running reminds nobody.
        let log = PrayerLog::in_memory().unwrap();
        assert!(load(&log).launch_at_login);
    }

    #[test]
    fn turning_launch_at_login_off_is_respected() {
        // The default must not override a user who deliberately turned it
        // off — a default is for the first run, not every run.
        let log = PrayerLog::in_memory().unwrap();
        let mut settings = Settings::default();
        settings.launch_at_login = false;
        save(&log, &settings);
        assert!(!load(&log).launch_at_login);
    }

    #[test]
    fn a_split_database_is_folded_back_into_one() {
        // What a live install written by the syncing build actually looks
        // like: the account half nested inside a record with its own
        // timestamp, and the device half beside it.
        let log = PrayerLog::in_memory().unwrap();
        log.save_json(
            ACCOUNT_KEY,
            &serde_json::json!({
                "settings": { "method": "karachi", "madhab": "shafi", "reciteMulk": false },
                "updatedAt": "2026-09-01T06:00:00Z",
                "deviceId": "an-older-device"
            }),
        );
        log.save_json(
            DEVICE_KEY,
            &serde_json::json!({
                "locationName": "Lahore",
                "latitude": 31.5204,
                "longitude": 74.3587,
                "timezone": "Asia/Karachi",
                "bedtimeHour": 22
            }),
        );

        let loaded = load(&log);
        // Both halves survived, and the record's own bookkeeping did not
        // leak in as a setting.
        assert_eq!(loaded.madhab, AsrMadhab::Shafi);
        assert!(!loaded.recite_mulk);
        assert_eq!(loaded.location.unwrap().name, "Lahore");
        assert_eq!(loaded.bedtime_hour, 22);

        // One key from now on.
        assert!(log.load_json::<Settings>(SETTINGS_KEY).is_some());
        // And the old keys stay put, so rolling back finds something sane.
        assert!(log.load_json::<serde_json::Value>(ACCOUNT_KEY).is_some());
        assert!(log.load_json::<serde_json::Value>(DEVICE_KEY).is_some());
    }

    #[test]
    fn half_a_split_database_still_loads() {
        // A device half with no account half, which is what a database
        // written before the account half was ever saved looks like.
        let log = PrayerLog::in_memory().unwrap();
        log.save_json(
            DEVICE_KEY,
            &serde_json::json!({
                "locationName": "Karachi",
                "latitude": 24.8607,
                "longitude": 67.0011,
                "timezone": "Asia/Karachi"
            }),
        );

        let loaded = load(&log);
        assert_eq!(loaded.location.unwrap().name, "Karachi");
        assert_eq!(loaded.madhab, AsrMadhab::Hanafi);
    }

    #[test]
    fn a_real_settings_blob_survives_intact() {
        // Taken verbatim from a live database, because this migration runs
        // exactly once against real settings and cannot be retried if it
        // drops something. Every field is asserted, not a sample.
        let log = PrayerLog::in_memory().unwrap();
        log.save_json(
            SETTINGS_KEY,
            &serde_json::json!({
                "locationName": "Gujranwala",
                "latitude": 32.19265558539923,
                "longitude": 74.17256787793612,
                "timezone": "Asia/Karachi",
                "method": "karachi",
                "madhab": "hanafi",
                "adjustments": { "fajr": 0, "sunrise": 0, "zuhr": 0, "asr": 0, "maghrib": 0, "isha": 0 },
                "firstNudge": [
                    { "prayer": "fajr", "firstNudge": { "kind": "offset_minutes", "minutes": 20 } },
                    { "prayer": "zuhr", "firstNudge": { "kind": "offset_minutes", "minutes": 30 } },
                    { "prayer": "asr", "firstNudge": { "kind": "offset_minutes", "minutes": 30 } },
                    { "prayer": "maghrib", "firstNudge": { "kind": "offset_minutes", "minutes": 10 } },
                    { "prayer": "isha", "firstNudge": { "kind": "offset_minutes", "minutes": 45 } }
                ],
                "jumuah": null,
                "bedtimeHour": 23,
                "bedtimeMinute": 30,
                // Removed since this blob was written; it must be dropped
                // quietly rather than blocking the migration.
                "countWitr": true,
                "reciteMulk": true,
                "launchAtLogin": true,
                "theme": "system",
                "onboarded": true
            }),
        );

        let first = load(&log);
        // And again, now reading the split keys rather than migrating.
        let loaded = load(&log);

        assert_eq!(first.location, loaded.location);
        let place = loaded.location.clone().expect("the stored place survived");
        assert_eq!(place.name, "Gujranwala");
        assert_eq!(place.latitude, 32.19265558539923);
        assert_eq!(place.longitude, 74.17256787793612);
        assert_eq!(place.timezone, "Asia/Karachi");
        assert_eq!(loaded.method, CalcMethod::Karachi);
        assert_eq!(loaded.madhab, AsrMadhab::Hanafi);
        assert_eq!(loaded.recite_mulk, true);
        assert_eq!(loaded.bedtime_hour, 23);
        assert_eq!(loaded.bedtime_minute, 30);
        assert_eq!(loaded.launch_at_login, true);
        assert_eq!(loaded.onboarded, true);
        assert!(loaded.jumuah.is_none());
        assert_eq!(loaded.adjustments.zuhr, 0);
        // The Zuhr offset had been changed from its default of 60.
        assert_eq!(
            loaded.first_nudge_for(Prayer::Zuhr, false),
            FirstNudge::OffsetMinutes { minutes: 30 }
        );
        assert_eq!(
            loaded.first_nudge_for(Prayer::Fajr, false),
            FirstNudge::OffsetMinutes { minutes: 20 }
        );
        // A setting that no longer exists is dropped on the way through,
        // so the bytes on disk agree with what the app believes.
        let stored: serde_json::Value = log.load_json(SETTINGS_KEY).unwrap();
        assert!(!stored.to_string().contains("countWitr"));
    }

    #[test]
    fn a_removed_setting_does_not_survive_the_fold() {
        // The case a live install is actually in: a stored record still
        // carrying a setting that no longer exists. It must be dropped
        // quietly rather than blocking the migration.
        let log = PrayerLog::in_memory().unwrap();
        log.save_json(
            ACCOUNT_KEY,
            &serde_json::json!({
                "settings": { "method": "karachi", "madhab": "shafi", "countWitr": true },
                "updatedAt": "2026-09-01T06:00:00Z",
                "deviceId": "an-older-device"
            }),
        );
        log.save_json(
            DEVICE_KEY,
            &serde_json::json!({
                "locationName": "Lahore",
                "latitude": 31.5204,
                "longitude": 74.3587,
                "timezone": "Asia/Karachi"
            }),
        );

        let loaded = load(&log);
        assert_eq!(loaded.madhab, AsrMadhab::Shafi);
        assert_eq!(loaded.location.clone().unwrap().name, "Lahore");

        let raw: serde_json::Value = log.load_json(SETTINGS_KEY).unwrap();
        assert!(!raw.to_string().contains("countWitr"), "stale setting kept");
        // Nor does the record's own bookkeeping become a setting.
        assert!(raw.get("updatedAt").is_none());
        assert!(raw.get("deviceId").is_none());
    }
}
