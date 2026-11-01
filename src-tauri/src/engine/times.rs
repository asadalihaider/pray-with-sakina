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
    let today = prayer_times(date, coordinates, params)?;
    let tomorrow = prayer_times(date + Duration::days(1), coordinates, params)?;

    Ok(DayTimes {
        fajr: today.time(SalahPrayer::Fajr),
        sunrise: today.time(SalahPrayer::Sunrise),
        zuhr: today.time(SalahPrayer::Dhuhr),
        asr: today.time(SalahPrayer::Asr),
        maghrib: today.time(SalahPrayer::Maghrib),
        isha: today.time(SalahPrayer::Isha),
        next_fajr: tomorrow.time(SalahPrayer::Fajr),
    })
}

/// How far to nudge a location when `salah` trips over its own rounding:
/// 0.15° of longitude moves every time by 36 seconds, 0.3° by 72.
const ROUNDING_NUDGES_DEGREES: [f64; 4] = [0.15, -0.15, 0.3, -0.3];

/// One day's times from `salah`, with its panics contained.
///
/// It panics in two ways. Near the poles the sun's angle math has no answer,
/// which is a real limit and surfaces as an error. But it also rounds to the
/// minute without carrying into the next day, so any time that lands in the
/// last 30 seconds before midnight UTC becomes hour 24 and panics — Fajr in
/// Pakistan does this a few days a year. Moving the location a hair east or
/// west pushes that time out of the half-minute, at the cost of at most a
/// minute or two of rounding on that one day. Both directions and two sizes
/// are tried, since `salah` works out sunrise for neighbouring days as well,
/// and a nudge that frees one can push another into the gap.
fn prayer_times(
    date: NaiveDate,
    coordinates: Coordinates,
    params: Parameters,
) -> Result<PrayerTimes, PrayerCalcError> {
    let attempt = |coordinates: Coordinates| {
        std::panic::catch_unwind(|| PrayerTimes::new(date, coordinates, params)).ok()
    };
    attempt(coordinates)
        .or_else(|| {
            ROUNDING_NUDGES_DEGREES.into_iter().find_map(|nudge| {
                attempt(Coordinates::new(
                    coordinates.latitude,
                    coordinates.longitude + nudge,
                ))
            })
        })
        .ok_or(PrayerCalcError::UnsupportedLocation)
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
    fn a_time_rounding_up_to_midnight_utc_still_computes() {
        // Gujranwala, where `salah` puts Fajr in the last half-minute before
        // midnight UTC on these days and then rounds it to hour 24.
        let coords = Coordinates::new(32.19255831157831, 74.1725743421067);
        for day in [
            NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(),
            // Sunrise rounds up to midnight here instead.
            NaiveDate::from_ymd_opt(2026, 5, 24).unwrap(),
            NaiveDate::from_ymd_opt(2026, 5, 25).unwrap(),
        ] {
            let times = compute_day_times(day, coords, karachi_hanafi_params()).unwrap();
            assert!(times.fajr < times.sunrise && times.isha < times.next_fajr);
        }
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
