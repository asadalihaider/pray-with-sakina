//! "Use my location" on macOS.
//!
//! A Mac has no GPS radio, so CoreLocation resolves position from nearby
//! Wi-Fi networks. That is easily accurate enough here: moving a few
//! kilometres shifts a prayer time by seconds.
//!
//! CLLocationManager wants a run loop, so the manager lives on the main
//! thread and is polled from the caller rather than driven by a delegate.
//! The timezone comes from the system clock rather than a lookup service:
//! a machine sitting at these coordinates is already set to the right one.
//! The city name comes from Apple's own geocoder, which already has the
//! coordinates, rather than a third party we would have to hand them to.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub timezone: String,
    /// None until the geocoder answers; the caller falls back to a generic
    /// label rather than blocking on a name it does not strictly need.
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocationError {
    Denied,
    Unavailable,
    TimedOut,
}

impl LocationError {
    pub fn message(self) -> &'static str {
        match self {
            LocationError::Denied => {
                "Location access is off for Sakina. Turn it on in System Settings → Privacy & Security → Location Services, or search for your city instead."
            }
            LocationError::Unavailable => {
                "Location services are unavailable on this Mac. Search for your city instead."
            }
            LocationError::TimedOut => {
                "Could not get a location in time. Search for your city instead."
            }
        }
    }
}

fn system_timezone() -> String {
    iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".to_string())
}

#[cfg(target_os = "macos")]
mod platform {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2_core_location::{CLAuthorizationStatus, CLLocationManager};

    use super::{DetectedLocation, LocationError};

    thread_local! {
        /// Main-thread only. The manager has to outlive the call that starts
        /// it, or updates stop before a fix arrives.
        static MANAGER: RefCell<Option<Retained<CLLocationManager>>> =
            const { RefCell::new(None) };
    }

    fn with_manager<T>(action: impl FnOnce(&CLLocationManager) -> T) -> T {
        MANAGER.with(|slot| {
            let mut slot = slot.borrow_mut();
            let manager = slot.get_or_insert_with(|| unsafe { CLLocationManager::new() });
            action(manager)
        })
    }

    pub fn start() {
        with_manager(|manager| unsafe {
            manager.requestWhenInUseAuthorization();
            manager.startUpdatingLocation();
        });
    }

    pub fn stop() {
        with_manager(|manager| unsafe { manager.stopUpdatingLocation() });
    }

    /// Asks Apple to name the coordinates. Best effort: a missing name just
    /// means the location shows as "Current location".
    // CLGeocoder is deprecated in favour of MapKit, which would pull in a
    // whole framework to name one point. It still works, so it stays until
    // there is a reason to move.
    #[allow(deprecated)]
    pub fn name_for(latitude: f64, longitude: f64) -> Option<String> {
        use std::sync::mpsc;

        use block2::RcBlock;
        use objc2_core_location::{CLGeocoder, CLLocation, CLPlacemark};
        use objc2::AnyThread;
        use objc2_foundation::{NSArray, NSError};

        let (sender, receiver) = mpsc::channel();
        let geocoder = unsafe { CLGeocoder::new() };
        let location =
            unsafe { CLLocation::initWithLatitude_longitude(CLLocation::alloc(), latitude, longitude) };

        let handler = RcBlock::new(
            move |placemarks: *mut NSArray<CLPlacemark>, _error: *mut NSError| {
                let name = (!placemarks.is_null())
                    .then(|| unsafe { &*placemarks })
                    .and_then(|list| list.firstObject())
                    .and_then(|place| unsafe {
                        place
                            .locality()
                            .or_else(|| place.subAdministrativeArea())
                            .or_else(|| place.administrativeArea())
                    })
                    .map(|text| text.to_string());
                let _ = sender.send(name);
            },
        );

        unsafe {
            geocoder.reverseGeocodeLocation_completionHandler(&location, RcBlock::as_ptr(&handler))
        };
        // The callback lands on the main run loop, so the caller must not be
        // holding it; this runs from the worker that polled for the fix.
        receiver
            .recv_timeout(std::time::Duration::from_secs(6))
            .ok()
            .flatten()
    }

    /// `Ok(None)` means "still waiting" — authorisation may not have been
    /// answered yet, and a first fix takes a moment.
    pub fn poll(timezone: String) -> Result<Option<DetectedLocation>, LocationError> {
        with_manager(|manager| {
            let status = unsafe { manager.authorizationStatus() };
            if status == CLAuthorizationStatus::Denied
                || status == CLAuthorizationStatus::Restricted
            {
                return Err(LocationError::Denied);
            }

            let Some(location) = (unsafe { manager.location() }) else {
                return Ok(None);
            };
            let coordinate = unsafe { location.coordinate() };
            if !unsafe { coordinate.is_valid() } {
                return Ok(None);
            }

            Ok(Some(DetectedLocation {
                latitude: coordinate.latitude,
                longitude: coordinate.longitude,
                timezone,
                name: None,
            }))
        })
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::{DetectedLocation, LocationError};

    pub fn start() {}
    pub fn stop() {}
    pub fn name_for(_latitude: f64, _longitude: f64) -> Option<String> {
        None
    }
    pub fn poll(_timezone: String) -> Result<Option<DetectedLocation>, LocationError> {
        Err(LocationError::Unavailable)
    }
}

pub use platform::{name_for, poll, start, stop};

pub fn timezone_now() -> String {
    system_timezone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_timezone_is_readable() {
        // Whatever this machine is set to, it has to parse as a real zone,
        // because prayer windows and bedtime are resolved through it.
        let name = timezone_now();
        assert!(name.parse::<chrono_tz::Tz>().is_ok(), "got {name}");
    }

    #[test]
    fn every_failure_explains_the_way_out() {
        for error in [
            LocationError::Denied,
            LocationError::Unavailable,
            LocationError::TimedOut,
        ] {
            assert!(error.message().contains("city"), "{error:?}");
        }
    }
}
