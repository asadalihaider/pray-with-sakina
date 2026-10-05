//! Pure prayer-time, window, nudge, and state logic — no I/O, no wall clock
//! of its own. Every function takes `now` (or a date) as a parameter so
//! tests can fix it and the caller (the scheduler) supplies the real clock.

mod nudges;
mod state;
mod times;
mod types;
mod windows;

pub use nudges::{first_nudge_time, next_nudge, nudge_cutoff};
pub use state::compute_state;
pub use times::{
    build_parameters, compute_day_times, DayTimes, PrayerAdjustments, PrayerCalcError,
};
pub use types::{FirstNudge, LoggedStatus, Prayer, PrayerState};
pub use windows::window_for;
