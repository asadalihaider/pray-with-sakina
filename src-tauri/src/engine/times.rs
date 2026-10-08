use chrono::{DateTime, Duration, NaiveDate, Utc};
use salah::{Configuration, Coordinates, Madhab, Method, Parameters, Prayer as SalahPrayer, PrayerTimes, TimeAdjustment};


/// Per-prayer manual minute adjustments, applied on top of the calculation
/// method, so users can match their masjid's posted times.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PrayerAdjustments {
    pub fajr: i64,
    pub sunrise: i64,
    pub zuhr: i64,
    pub asr: i64,
    pub maghrib: i64,
    pub isha: i64,
}

/// The calculated clock times for a single calendar day, plus the following
/// day's Fajr, since the Isha window runs past midnight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DayTimes {
    pub fajr: DateTime<Utc>,
    pub sunrise: DateTime<Utc>,
    pub zuhr: DateTime<Utc>,
    pub asr: DateTime<Utc>,
    pub maghrib: DateTime<Utc>,
    pub isha: DateTime<Utc>,
    pub next_fajr: DateTime<Utc>,
}

pub fn build_parameters(
    method: Method,
    madhab: Madhab,
    adjustments: PrayerAdjustments,
) -> Parameters {
    let mut params = Configuration::with(method, madhab);
    params.adjustments = TimeAdjustment::new(
        adjustments.fajr,
        adjustments.sunrise,
        adjustments.zuhr,
        adjustments.asr,
        adjustments.maghrib,
        adjustments.isha,
    );
    params
}

/// The `salah` crate's solar angle math can panic outright for a handful of
/// extreme high-latitude, near-solstice locations (verified against
/// Reykjavik in June) instead of returning an error. We can't fix the crate
/// from here, so we contain the panic and surface it as a normal error —
/// this function must never be allowed to take the whole app down just
/// because a user's coordinates land in that edge case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrayerCalcError {
    UnsupportedLocation,
    /// No place has been chosen yet, so there is nothing to calculate from.
    /// Not a failure so much as a question that has not been answered.
    LocationUnset,
}

impl PrayerCalcError {
    /// Worth saying out loud rather than debug-printing: both of these reach
    /// the user, and one of them is fixable by them.
    pub fn message(&self) -> &'static str {
        match self {
            Self::UnsupportedLocation => {
                "Prayer times cannot be worked out for this location."
            }
            Self::LocationUnset => "No location set yet.",
        }
    }
}

/// Computes a calendar day's prayer times for a location, including the
/// following day's Fajr (needed as the Isha window's end).
pub fn compute_day_times(
    date: NaiveDate,
    coordinates: Coordinates,
    params: Parameters,
) -> Result<DayTimes, PrayerCalcError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let today = PrayerTimes::new(date, coordinates, params);
        let tomorrow = PrayerTimes::new(date + Duration::days(1), coordinates, params);

        DayTimes {
            fajr: today.time(SalahPrayer::Fajr),
            sunrise: today.time(SalahPrayer::Sunrise),
            zuhr: today.time(SalahPrayer::Dhuhr),
            asr: today.time(SalahPrayer::Asr),
            maghrib: today.time(SalahPrayer::Maghrib),
            isha: today.time(SalahPrayer::Isha),
            next_fajr: tomorrow.time(SalahPrayer::Fajr),
        }
    }))
    .map_err(|_| PrayerCalcError::UnsupportedLocation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn karachi_hanafi_params() -> Parameters {
        build_parameters(Method::Karachi, Madhab::Hanafi, PrayerAdjustments::default())
    }

    #[test]
    fn karachi_times_are_in_order() {
        let coords = Coordinates::new(24.8607, 67.0011);
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let day = compute_day_times(date, coords, karachi_hanafi_params()).unwrap();

        assert!(day.fajr < day.sunrise);
        assert!(day.sunrise < day.zuhr);
        assert!(day.zuhr < day.asr);
        assert!(day.asr < day.maghrib);
        assert!(day.maghrib < day.isha);
        assert!(day.isha < day.next_fajr);
    }

    #[test]
    fn hanafi_asr_is_later_than_shafi_asr() {
        let coords = Coordinates::new(24.8607, 67.0011);
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();

        let hanafi = compute_day_times(date, coords, karachi_hanafi_params()).unwrap();
        let shafi = compute_day_times(
            date,
            coords,
            build_parameters(Method::Karachi, Madhab::Shafi, PrayerAdjustments::default()),
        )
        .unwrap();

        assert!(hanafi.asr > shafi.asr);
    }

    #[test]
    fn manual_adjustment_shifts_the_time() {
        let coords = Coordinates::new(24.8607, 67.0011);
        let date = NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();

        let base = compute_day_times(date, coords, karachi_hanafi_params()).unwrap();
        let adjusted = compute_day_times(
            date,
            coords,
            build_parameters(
                Method::Karachi,
                Madhab::Hanafi,
                PrayerAdjustments {
                    maghrib: 7,
                    ..Default::default()
                },
            ),
        )
        .unwrap();

        assert_eq!(adjusted.maghrib - base.maghrib, Duration::minutes(7));
    }

    #[test]
    fn moderate_high_latitude_city_still_produces_ordered_times() {
        // Oslo, Norway (~59.9N) in winter — high latitude, but not the
        // near-polar-day extreme, so the crate resolves it normally.
        let coords = Coordinates::new(59.9139, 10.7522);
        let date = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let day = compute_day_times(date, coords, karachi_hanafi_params()).unwrap();

        assert!(day.fajr < day.sunrise);
        assert!(day.sunrise < day.zuhr);
        assert!(day.zuhr < day.asr);
        assert!(day.asr < day.maghrib);
        assert!(day.maghrib < day.isha);
        assert!(day.isha < day.next_fajr);
    }

    #[test]
    fn near_polar_day_location_is_a_graceful_error_not_a_crash() {
        // Reykjavik in midsummer: the sun barely dips below the horizon,
        // which crashes the `salah` crate's solar angle math outright
        // (confirmed against salah 0.7.6). The engine must contain that
        // and hand back an error instead of taking the app down.
        let coords = Coordinates::new(64.1466, -21.9426);
        let date = NaiveDate::from_ymd_opt(2026, 6, 21).unwrap();

        assert_eq!(
            compute_day_times(date, coords, karachi_hanafi_params()),
            Err(PrayerCalcError::UnsupportedLocation)
        );
    }
}
