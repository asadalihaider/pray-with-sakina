use serde::{Deserialize, Serialize};

/// The five obligatory prayers. Sunrise is tracked separately (it closes the
/// Fajr window but is never itself nudged or logged).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prayer {
    Fajr,
    Zuhr,
    Asr,
    Maghrib,
    Isha,
}

impl Prayer {
    pub const ALL: [Prayer; 5] = [
        Prayer::Fajr,
        Prayer::Zuhr,
        Prayer::Asr,
        Prayer::Maghrib,
        Prayer::Isha,
    ];
}

/// How a prayer's first nudge is scheduled: a fixed offset after the prayer
/// starts, or an exact jamaat time the user has entered (also used for
/// Friday Jumu'ah, which is just Zuhr with its own jamaat time).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FirstNudge {
    /// A struct variant, not a newtype one: serde cannot internally tag a
    /// newtype variant wrapping a bare integer, and settings are stored as
    /// tagged JSON.
    OffsetMinutes { minutes: i64 },
    JamaatTime { hour: u32, minute: u32 },
}

/// A prayer's final, user-editable state.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrayerState {
    Upcoming,
    Active,
    Prayed { at: chrono::DateTime<chrono::Utc> },
    Unlogged,
    Missed,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoggedStatus {
    Prayed,
    Missed,
}
