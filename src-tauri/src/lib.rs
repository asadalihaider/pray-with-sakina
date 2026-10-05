mod engine;
mod icon;
mod location;
mod scheduler;
mod settings;
mod stats;
mod store;
mod today;

use chrono::Utc;
use tauri::{
    image::Image,
    Emitter,
    tray::{MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, PhysicalPosition, WebviewWindow,
};

use engine::{LoggedStatus, Prayer};
use scheduler::Scheduler;
use settings::Settings;
use store::PrayerLog;
use today::TodayView;

const TRAY_ID: &str = "sakina";
const ICON_SIZE: u32 = 44;
/// The plan's scheduler cadence: a Rust loop, not a web view timer, so the
/// OS cannot quietly throttle it while the app is idle.
const TICK_SECONDS: u64 = 30;

struct AppState {
    settings: std::sync::RwLock<Settings>,
    log: PrayerLog,
    scheduler: Scheduler,
    /// Where the user last left the panel. Empty until it is dragged, so the
    /// first open is centred.
    last_position: std::sync::Mutex<Option<PhysicalPosition<i32>>>,
    /// Set while a system dialog — the location permission prompt — is
    /// expected to take focus. Without it the panel treats that prompt as a
    /// click elsewhere and hides itself mid-request.
    hold_open: std::sync::atomic::AtomicBool,
    /// When an outside click last closed the panel.
    ///
    /// A click on the tray icon is *also* an outside click, so the monitor
    /// hides on mouse-down and the tray's own handler would reopen on
    /// mouse-up — the panel visibly flickering shut and straight back open.
    /// This is how the second half learns the first half already acted.
    dismissed_at: std::sync::Mutex<Option<std::time::Instant>>,
    /// How long after a system dialog closes the panel still ignores clicks.
    ///
    /// The dialog's own click reaches our run loop *after* the blocking call
    /// that raised it has returned, so releasing the hold the instant the
    /// command finishes still lets that click through — and the panel shut
    /// the moment the user pressed Allow.
    grace_until: std::sync::Mutex<Option<std::time::Instant>>,
}

impl AppState {
    fn new(log: PrayerLog) -> Self {
        let settings = settings::load(&log);
        Self {
            settings: std::sync::RwLock::new(settings),
            log,
            scheduler: Scheduler::default(),
            last_position: std::sync::Mutex::new(None),
            hold_open: std::sync::atomic::AtomicBool::new(false),
            dismissed_at: std::sync::Mutex::new(None),
            grace_until: std::sync::Mutex::new(None),
        }
    }
}

#[tauri::command]
fn get_today(state: tauri::State<AppState>) -> Result<TodayView, String> {
    today::build_view(Utc::now(), &state.settings.read().unwrap(), &state.log).map_err(|error| format!("{error:?}"))
}

#[tauri::command]
fn log_prayer(
    prayer: Prayer,
    status: LoggedStatus,
    app: AppHandle,
    state: tauri::State<AppState>,
) -> Result<TodayView, String> {
    let now = Utc::now();
    let date = today::display_date(now, &state.settings.read().unwrap())
        .map_err(|error| format!("{error:?}"))?;
    state.log.set(date, prayer, status, now);

    refresh_tray_title(&app);
    today::build_view(now, &state.settings.read().unwrap(), &state.log).map_err(|error| format!("{error:?}"))
}

#[tauri::command]
fn get_review(state: tauri::State<AppState>) -> Result<Vec<today::ReviewItem>, String> {
    today::pending_review(Utc::now(), &state.settings.read().unwrap(), &state.log)
        .map_err(|error| format!("{error:?}"))
}

/// Logs a prayer from an earlier day, which is how the morning review and
/// the history screen correct the record.
#[tauri::command]
fn log_past(
    date: String,
    prayer: Prayer,
    status: LoggedStatus,
    app: AppHandle,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let date: chrono::NaiveDate = date.parse().map_err(|_| "unreadable date".to_string())?;
    state.log.set(date, prayer, status, Utc::now());
    refresh_tray_title(&app);
    Ok(())
}

#[tauri::command]
fn get_settings(state: tauri::State<AppState>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// Saving settings can move every prayer time, so the tray title is
/// refreshed straight away rather than waiting up to 30s for the next tick.
#[tauri::command]
fn save_settings(
    settings: Settings,
    app: AppHandle,
    state: tauri::State<AppState>,
) -> Result<Settings, String> {
    apply_autostart(&app, settings.launch_at_login);
    apply_appearance(&app, settings.theme);
    settings::save(&state.log, &settings);
    *state.settings.write().unwrap() = settings.clone();
    refresh_tray_title(&app);
    Ok(settings)
}

/// Fires a reminder on demand, so the notification path can be checked
/// without waiting for a prayer window. Debug builds only — it is a
/// developer's tool, not a feature.
#[tauri::command]
fn preview_reminder(app: AppHandle) {
    #[cfg(debug_assertions)]
    preview_nudge(&app);
    #[cfg(not(debug_assertions))]
    let _ = app;
}

/// Keeps the panel open across a system dialog.
///
/// The location permission prompt belongs to another process, so allowing
/// it is an outside click — and the outside-click monitor would close the
/// panel out from under the very action that raised the prompt, which looks
/// exactly like the app quitting.
fn hold_panel_open(state: &AppState) {
    state
        .hold_open
        .store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Releases the hold, and then ignores clicks for a moment longer.
///
/// The dialog's own click reaches our run loop *after* the blocking call
/// that raised it has returned, so releasing the hold the instant the
/// command finishes still lets that click through.
fn release_panel(state: &AppState) {
    state
        .hold_open
        .store(false, std::sync::atomic::Ordering::Relaxed);
    *state.grace_until.lock().unwrap() =
        Some(std::time::Instant::now() + std::time::Duration::from_millis(600));
    // The dialog's own click must not count as a dismissal either.
    *state.dismissed_at.lock().unwrap() = None;
}


/// The vibrancy material is drawn by AppKit, not the web view, so forcing a
/// theme in CSS alone would leave a light panel sitting on a dark frosted
/// backing. Setting the window's appearance keeps the two in step.
#[cfg(target_os = "macos")]
fn apply_appearance(app: &AppHandle, theme: settings::Theme) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    };

    let Some(window) = popover(app) else {
        return;
    };
    let Some(ns_window) = as_ns_window(&window) else {
        return;
    };

    let name = match theme {
        settings::Theme::System => None,
        settings::Theme::Light => Some(unsafe { NSAppearanceNameAqua }),
        settings::Theme::Dark => Some(unsafe { NSAppearanceNameDarkAqua }),
    };
    let appearance = name.and_then(|name| NSAppearance::appearanceNamed(name));
    ns_window.setAppearance(appearance.as_deref());
}

#[cfg(not(target_os = "macos"))]
fn apply_appearance(_app: &AppHandle, _theme: settings::Theme) {}

fn apply_autostart(app: &AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;

    let manager = app.autolaunch();
    let _ = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
}

/// Polls CoreLocation on the main thread until a fix arrives. The manager
/// needs a run loop, so the work has to hop back to the main thread each
/// time rather than blocking it for the whole wait.
#[tauri::command]
async fn detect_location(app: AppHandle) -> Result<location::DetectedLocation, String> {
    const ATTEMPTS: u32 = 20;
    const GAP: std::time::Duration = std::time::Duration::from_millis(500);

    let timezone = location::timezone_now();
    hold_panel_open(&app.state::<AppState>());
    let starter = app.clone();
    let _ = starter.run_on_main_thread(location::start);

    let releaser = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        for _ in 0..ATTEMPTS {
            let (sender, receiver) = std::sync::mpsc::channel();
            let zone = timezone.clone();
            if app
                .run_on_main_thread(move || {
                    let _ = sender.send(location::poll(zone));
                })
                .is_err()
            {
                return Err(location::LocationError::Unavailable);
            }

            match receiver.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(Ok(Some(mut found))) => {
                    let _ = app.run_on_main_thread(location::stop);
                    found.name = location::name_for(found.latitude, found.longitude);
                    return Ok(found);
                }
                Ok(Err(error)) => {
                    let _ = app.run_on_main_thread(location::stop);
                    return Err(error);
                }
                _ => std::thread::sleep(GAP),
            }
        }
        let _ = app.run_on_main_thread(location::stop);
        Err(location::LocationError::TimedOut)
    })
    .await;

    release_panel(&releaser.state::<AppState>());

    match result {
        Ok(Ok(found)) => Ok(found),
        // The bare message cannot tell "macOS never asked" apart from
        // "macOS refused", and those need different things from the user.
        Ok(Err(error)) => Err(format!(
            "{} (location access: {})",
            error.message(),
            location::status_name()
        )),
        Err(_) => Err(location::LocationError::Unavailable.message().to_string()),
    }
}

#[tauri::command]
fn get_stats(
    year: i32,
    month: u32,
    state: tauri::State<AppState>,
) -> Result<stats::StatsView, String> {
    stats::month_stats(Utc::now(), &state.settings.read().unwrap(), &state.log, year, month)
        .map_err(|error| format!("{error:?}"))
}

#[tauri::command]
fn get_qaza(state: tauri::State<AppState>) -> stats::QazaView {
    stats::qaza(&state.log)
}

#[tauri::command]
fn add_madeup(prayer: Prayer, state: tauri::State<AppState>) -> stats::QazaView {
    state.log.add_madeup(prayer, Utc::now());
    stats::qaza(&state.log)
}

#[tauri::command]
fn set_backlog(prayer: Prayer, count: i64, state: tauri::State<AppState>) -> stats::QazaView {
    state.log.set_backlog(prayer, count, Utc::now());
    stats::qaza(&state.log)
}

/// Surah Mulk is a daily habit rather than a prayer, so it gets its own
/// command instead of overloading `log_past` with a sixth pseudo-prayer.
#[tauri::command]
fn log_mulk(
    date: String,
    recited: bool,
    state: tauri::State<AppState>,
) -> Result<(), String> {
    let date: chrono::NaiveDate = date.parse().map_err(|_| "unreadable date".to_string())?;
    state.log.set_mulk(date, recited, Utc::now());
    Ok(())
}

#[tauri::command]
fn finish_review(state: tauri::State<AppState>) -> Result<(), String> {
    let today = today::display_date(Utc::now(), &state.settings.read().unwrap())
        .map_err(|error| format!("{error:?}"))?;
    state.log.mark_reviewed(today);
    Ok(())
}

fn format_remaining(milliseconds: i64) -> String {
    let minutes = milliseconds / 60_000;
    if minutes >= 60 {
        let hours = minutes / 60;
        let rest = minutes % 60;
        if rest == 0 {
            format!("{hours}h")
        } else {
            format!("{hours}h {rest}m")
        }
    } else if minutes >= 1 {
        format!("{minutes}m")
    } else {
        "<1m".to_string()
    }
}

/// Logging a prayer marks it but must not blank the countdown: how long is
/// left in the window is still worth knowing once you have prayed.
fn tray_title(view: &TodayView) -> String {
    let focus = &view.focus;
    let remaining = format_remaining(focus.remaining_ms);
    match (focus.active, focus.prayed_at.is_some()) {
        (true, true) => format!("{} · {} ✓", focus.label, remaining),
        (true, false) => format!("{} · {}", focus.label, remaining),
        (false, _) => format!("{} in {}", focus.label, remaining),
    }
}

fn refresh_tray_title(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let state = app.state::<AppState>();
    let title = match today::build_view(Utc::now(), &state.settings.read().unwrap(), &state.log) {
        Ok(view) => tray_title(&view),
        // A location the prayer time maths cannot resolve must not leave a
        // stale countdown sitting in the menu bar.
        Err(_) => "—".to_string(),
    };
    let _ = tray.set_title(Some(title));
}

/// macOS ties a window to the Space it was created on, so a popover built on
/// desktop 1 does not appear while you are on another desktop. These flags
/// fix that case, and Tauri's own `set_visible_on_all_workspaces` sets only
/// the first of them.
///
/// KNOWN GAP: this is still not enough to draw over a *fullscreen* app, even
/// with `FullScreenAuxiliary` and a window level above the fullscreen window.
/// The likely reason is that a plain `NSWindow` owned by an Accessory app
/// cannot take key status over a fullscreen Space — the usual cure is to make
/// the popover an `NSPanel` with the non-activating style mask, which Tauri
/// does not create natively (the `tauri-nspanel` crate exists for exactly
/// this). Revisit before the beta.
#[cfg(target_os = "macos")]
fn float_over_every_space(window: &WebviewWindow) {
    use objc2_app_kit::{NSPopUpMenuWindowLevel, NSWindow, NSWindowCollectionBehavior};

    let Ok(pointer) = window.ns_window() else {
        return;
    };
    // Safety: `ns_window` hands back the NSWindow backing this window, which
    // outlives the borrow because the window owns it.
    let ns_window: &NSWindow = unsafe { &*(pointer as *const NSWindow) };
    ns_window.setCollectionBehavior(
        ns_window.collectionBehavior()
            | NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    ns_window.setLevel(NSPopUpMenuWindowLevel);
}

/// Fires the system notification for a due nudge. Clicking it opens the
/// popover, where the prayer is actually logged.
///
/// `send_notification` blocks until the user acts on or dismisses the
/// notification, so it runs on its own thread rather than stalling the
/// scheduler tick.
#[cfg(target_os = "macos")]
fn notify(app: &AppHandle, due: &scheduler::DueNudge) {
    let title = format!("{} · {} left", due.label, format_remaining(due.remaining_ms));
    let body = if due.ending_soon {
        "Ending soon — have you prayed?"
    } else {
        "Have you prayed?"
    };
    // NSUserNotificationCenter silently withholds the banner when the app
    // delivering it is frontmost, and mac-notification-sys does not
    // implement shouldPresentNotification to override that. Stepping out of
    // the way first is what makes the reminder actually appear rather than
    // landing straight in Notification Center.
    resign_active(app);

    let app = app.clone();
    let prayer = due.prayer;

    std::thread::spawn(move || {
        use mac_notification_sys::{MainButton, Notification, NotificationResponse};

        // Without `wait_for_click` the call is fire-and-forget and always
        // reports `None`, so neither the click nor the button reaches us.
        let mut options = Notification::new();
        options.wait_for_click(true);
        options.main_button(MainButton::SingleAction("Prayed"));

        match mac_notification_sys::send_notification(&title, None, body, Some(&options)) {
            // Logged straight from the notification: the quickest path is
            // not opening the app at all.
            Ok(NotificationResponse::ActionButton(_)) => {
                let now = Utc::now();
                let state = app.state::<AppState>();
                let date = today::display_date(now, &state.settings.read().unwrap());
                if let Ok(date) = date {
                    state.log.set(date, prayer, LoggedStatus::Prayed, now);
                }
                refresh_tray_title(&app);
            }
            Ok(NotificationResponse::Click) => {
                let handle = app.clone();
                let _ = app.run_on_main_thread(move || show_popover(&handle));
            }
            _ => {}
        }
    });
}

#[cfg(not(target_os = "macos"))]
fn notify(_app: &AppHandle, _due: &scheduler::DueNudge) {}

#[cfg(target_os = "macos")]
fn resign_active(app: &AppHandle) {
    let _ = app.run_on_main_thread(|| {
        use objc2_app_kit::NSApplication;

        let Some(marker) = objc2::MainThreadMarker::new() else {
            return;
        };
        let ns_app = NSApplication::sharedApplication(marker);
        if ns_app.isActive() {
            ns_app.deactivate();
        }
    });
}

/// Development-only: fires a reminder immediately, since a real one can be
/// most of an hour away.
#[cfg(debug_assertions)]
fn preview_nudge(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Ok(view) = today::build_view(Utc::now(), &state.settings.read().unwrap(), &state.log) else {
        return;
    };
    let focus = view.focus;

    notify(
        app,
        &scheduler::DueNudge {
            prayer: focus.prayer,
            label: focus.label,
            arabic: focus.arabic,
            remaining_ms: focus.remaining_ms,
            window_fraction: focus.window_fraction,
            ending_soon: focus.remaining_ms < 20 * 60_000,
        },
    );
}

/// Runs the window from transparent to opaque through AppKit's own animator,
/// so the fade is driven by the compositor rather than by the web view.
#[cfg(target_os = "macos")]
fn as_ns_window(window: &WebviewWindow) -> Option<&objc2_app_kit::NSWindow> {
    let pointer = window.ns_window().ok()?;
    // Safety: the window owns the NSWindow this pointer refers to.
    Some(unsafe { &*(pointer as *const objc2_app_kit::NSWindow) })
}

#[cfg(target_os = "macos")]
fn hide_for_fade(window: &WebviewWindow) {
    if let Some(ns_window) = as_ns_window(window) {
        ns_window.setAlphaValue(0.0);
    }
}

#[cfg(target_os = "macos")]
fn fade_in(window: &WebviewWindow) {
    use objc2_app_kit::{NSAnimatablePropertyContainer, NSAnimationContext};

    let Some(ns_window) = as_ns_window(window) else {
        return;
    };
    NSAnimationContext::beginGrouping();
    NSAnimationContext::currentContext().setDuration(0.16);
    ns_window.animator().setAlphaValue(1.0);
    NSAnimationContext::endGrouping();
}

fn popover(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

/// Centres the panel the first time, then reopens wherever the user last
/// dragged it to.
/// Closes the panel when the user clicks outside the app.
///
/// A global monitor sees only events that go to *other* applications, which
/// is exactly the definition wanted here: a click elsewhere dismisses the
/// panel, and nothing else does.
///
/// Focus was the wrong signal. An auto-hidden menu bar takes focus merely by
/// revealing itself as the pointer nears the top of the screen, so the panel
/// closed while the user was still reaching for the tray icon — and
/// suppressing that left it visible but unfocused, a state it could never
/// blur out of again. A click is the event the user actually performs.
#[cfg(target_os = "macos")]
fn dismiss_on_outside_click(app: AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask};

    let mask = NSEventMask::LeftMouseDown
        | NSEventMask::RightMouseDown
        | NSEventMask::OtherMouseDown;

    let handler = RcBlock::new(move |_event: std::ptr::NonNull<NSEvent>| {
        let state = app.state::<AppState>();
        if state
            .hold_open
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        if state
            .grace_until
            .lock()
            .unwrap()
            .is_some_and(|until| std::time::Instant::now() < until)
        {
            return;
        }
        if let Some(window) = popover(&app) {
            if window.is_visible().unwrap_or(false) {
                remember_position(&app, &window);
                let _ = window.hide();
                *state.dismissed_at.lock().unwrap() = Some(std::time::Instant::now());
            }
        }
    });

    // The returned token must outlive the monitor; letting it drop would
    // unregister it immediately and silently.
    let token = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(mask, &handler);
    std::mem::forget(token);
    std::mem::forget(handler);
}

/// The tray's Quit menu, shown on a double click.
///
/// Built and popped here rather than handed to `NSStatusItem`, because a
/// menu attached to the status item is one AppKit will open by itself on a
/// single click — see the tray builder. The item's action is `terminate:`
/// with no target, so it travels the responder chain to NSApplication and
/// needs no class of our own.
#[cfg(target_os = "macos")]
fn show_quit_menu(at: tauri::PhysicalPosition<f64>) {
    use objc2::{sel, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSMenu, NSMenuItem, NSScreen};
    use objc2_foundation::{NSPoint, NSString};

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    unsafe {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Sakina"));
        let item = NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Quit Sakina"),
            Some(sel!(terminate:)),
            &NSString::from_str("q"),
        );
        menu.addItem(&item);

        // Cocoa's screen coordinates put the origin at the bottom left,
        // while the tray reports the position from the top. Flipping needs
        // the *primary* screen's height, which is what that origin belongs
        // to, not whichever display the pointer happens to be on.
        let flipped = NSScreen::screens(mtm)
            .firstObject()
            .map(|screen| screen.frame().size.height - at.y)
            .unwrap_or(at.y);

        menu.popUpMenuPositioningItem_atLocation_inView(None, NSPoint::new(at.x, flipped), None);
    }
}

fn place_popover(app: &AppHandle, window: &WebviewWindow) {
    if let Some(previous) = *app.state::<AppState>().last_position.lock().unwrap() {
        let _ = window.set_position(previous);
        return;
    }

    let Some(monitor) = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
    else {
        return;
    };
    let Ok(size) = window.outer_size() else {
        return;
    };

    let screen = monitor.size();
    let origin = monitor.position();
    let _ = window.set_position(PhysicalPosition::new(
        origin.x + (screen.width as i32 - size.width as i32) / 2,
        origin.y + (screen.height as i32 - size.height as i32) / 2,
    ));
}

/// Remembers where the panel was before it closes, so it comes back to the
/// same spot rather than jumping to the middle again.
fn remember_position(app: &AppHandle, window: &WebviewWindow) {
    if let Ok(position) = window.outer_position() {
        app.state::<AppState>()
            .last_position
            .lock()
            .unwrap()
            .replace(position);
    }
}

fn show_popover(app: &AppHandle) {
    let Some(window) = popover(app) else {
        return;
    };

    // Placed while still transparent, so the move is never seen. Positioning
    // a hidden window does not reliably stick, hence show first.
    #[cfg(target_os = "macos")]
    hide_for_fade(&window);
    let _ = window.show();
    place_popover(app, &window);
    #[cfg(target_os = "macos")]
    fade_in(&window);

    let _ = window.set_focus();
    // Lets the web view replay its entrance animation on every open, not
    // just the first time it is created.
    let _ = window.emit_to("main", "popover-shown", ());
}

fn toggle_popover(app: &AppHandle) {
    let Some(window) = popover(app) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        remember_position(app, &window);
        let _ = window.hide();
        return;
    }

    // The panel may have been open a moment ago and closed by this very
    // click: the outside-click monitor fires on mouse-down, this runs on
    // mouse-up. Reopening now would undo what the user just did, which they
    // would see as a flicker and an icon that refuses to close.
    //
    // A window is generous enough to cover a slow click and short enough
    // that a deliberate second click still reopens.
    let state = app.state::<AppState>();
    let just_dismissed = state
        .dismissed_at
        .lock()
        .unwrap()
        .take()
        .is_some_and(|at| at.elapsed() < std::time::Duration::from_millis(400));
    if just_dismissed {
        return;
    }

    show_popover(app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_today,
            log_prayer,
            get_review,
            log_past,
            finish_review,
            get_stats,
            get_qaza,
            add_madeup,
            set_backlog,
            get_settings,
            save_settings,
            preview_reminder,
            log_mulk,
            detect_location
        ])
        // Builder::setup replaces any previous hook rather than chaining, so
        // everything that runs at startup has to live in this one closure.
        .setup(|app| {
            let path = app
                .path()
                .app_data_dir()
                .map(|directory| directory.join("sakina.db"));

            // Losing the database must not cost the user the app: without it
            // logging still works for this session, it just is not kept.
            let log = path
                .ok()
                .and_then(|path| PrayerLog::open(&path).ok())
                .or_else(|| PrayerLog::in_memory().ok())
                .expect("an in-memory database is always available");

            app.manage(AppState::new(log));
            let state = app.state::<AppState>();
            let (startup_theme, onboarded, launch_at_login) = {
                let settings = state.settings.read().unwrap();
                (settings.theme, settings.onboarded, settings.launch_at_login)
            };

            // Applied on every launch, not only when the setting is changed.
            // It used to be applied only from `save_settings`, which meant a
            // fresh install showed the switch on and registered nothing —
            // the app would not actually start at login until the user
            // opened Settings and toggled it. Re-applying also repoints the
            // launch agent after the app is moved or reinstalled, since the
            // agent holds an absolute path.
            apply_autostart(&app.handle().clone(), launch_at_login);

            // A menu bar app, not a windowed one: no Dock icon, no app switcher.
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                // Only takes effect for a bundled .app; an unbundled `tauri
                // dev` binary is not registered with Launch Services, so
                // notifications there are attributed elsewhere.
                let _ = mac_notification_sys::set_application(
                    &app.config().identifier,
                );
            }

            if let Some(window) = app.get_webview_window("main") {
                #[cfg(target_os = "macos")]
                float_over_every_space(&window);

                #[cfg(target_os = "macos")]
                let _ = window_vibrancy::apply_vibrancy(
                    &window,
                    window_vibrancy::NSVisualEffectMaterial::Popover,
                    None,
                    Some(14.0),
                );

                // Dismissal is driven by clicks, not focus — see
                // `dismiss_on_outside_click`.
                #[cfg(target_os = "macos")]
                dismiss_on_outside_click(app.handle().clone());
            }

            // No menu is attached to the tray icon, deliberately.
            //
            // `tray-icon` 0.24 calls `NSStatusItem::setMenu` as soon as one
            // is given and only *later* decides whether to pop it, relying on
            // a custom view intercepting `mouseDown:` to stop AppKit showing
            // it first. macOS 27 no longer lets that view win, so every left
            // click opened the Quit menu instead of the panel.
            //
            // Upstream 0.25 fixes this by attaching the menu only for the
            // instant a click needs it, but stable Tauri pins ^0.24. Owning
            // no menu at all reaches the same place and cannot break again:
            // Quit lives in Settings, where the rest of the app already is.
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(Image::new_owned(
                    icon::star_rgba(ICON_SIZE),
                    ICON_SIZE,
                    ICON_SIZE,
                ))
                .icon_as_template(true)
                .on_tray_icon_event(|tray, event| match event {
                    TrayIconEvent::Click {
                        button_state: MouseButtonState::Up,
                        ..
                    } => toggle_popover(tray.app_handle()),
                    // A double click arrives *after* two ordinary clicks, so
                    // the panel has already opened and closed by now. Hiding
                    // it again is harmless and covers the case where an odd
                    // number of clicks left it open.
                    TrayIconEvent::DoubleClick { position, .. } => {
                        let app = tray.app_handle().clone();
                        let opener = app.clone();
                        let _ = app.run_on_main_thread(move || {
                            if let Some(window) = opener.get_webview_window("main") {
                                let _ = window.hide();
                            }
                            #[cfg(target_os = "macos")]
                            show_quit_menu(position);
                        });
                    }
                    _ => {}
                })
                .build(app)?;

            apply_appearance(&app.handle().clone(), startup_theme);

            // On a first run the tray star alone is too quiet an invitation,
            // so the panel opens itself once to introduce the app.
            if !onboarded {
                let handle = app.handle().clone();
                let opener = handle.clone();
                let _ = handle.run_on_main_thread(move || show_popover(&opener));
            }

            let handle = app.handle().clone();
            std::thread::spawn(move || loop {
                refresh_tray_title(&handle);

                let state = handle.state::<AppState>();
                if let Ok(Some(due)) =
                    state
                        .scheduler
                        .poll(Utc::now(), &state.settings.read().unwrap(), &state.log)
                {
                    notify(&handle, &due);
                }

                std::thread::sleep(std::time::Duration::from_secs(TICK_SECONDS));
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_logged_prayer_keeps_its_countdown_in_the_menu_bar() {
        use chrono::Utc;
        let settings = Settings::default();
        let log = PrayerLog::in_memory().unwrap();
        let now = Utc::now();

        let before = today::build_view(now, &settings, &log).unwrap();
        let title_before = tray_title(&before);

        if before.focus.active {
            let date = today::display_date(now, &settings).unwrap();
            log.set(date, before.focus.prayer, LoggedStatus::Prayed, now);
            let after = today::build_view(now, &settings, &log).unwrap();
            let title_after = tray_title(&after);

            assert!(title_after.ends_with('✓'));
            // The countdown itself survives; only a tick is added.
            assert!(title_after.starts_with(title_before.as_str()));
        }
    }

    #[test]
    fn remaining_time_reads_the_way_the_menu_bar_needs_it() {
        assert_eq!(format_remaining(72 * 60_000), "1h 12m");
        assert_eq!(format_remaining(120 * 60_000), "2h");
        assert_eq!(format_remaining(42 * 60_000), "42m");
        assert_eq!(format_remaining(30_000), "<1m");
    }
}
