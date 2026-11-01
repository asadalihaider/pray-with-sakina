//! Whether anyone is at the Mac.
//!
//! A menu bar app runs all the time, including while nobody is there to see
//! it: the screen locked, the display asleep, the lid shut on a Mac that
//! stays awake on power. A reminder posted then lights up an empty lock
//! screen and is stale by the time anyone reads it. So the scheduler asks
//! this first, and saves what it has to say for when someone is back.
//!
//! Away is any of four things macOS reports, tracked separately because
//! they overlap and end in different orders: the Mac going to sleep, its
//! displays going to sleep, the screen locking, and another user taking
//! over the session. Someone is back only once every one of them has ended.
//! Real sleep also freezes the app outright, so the flag matters most for
//! the moment it thaws, before the screen has been unlocked.

use std::sync::atomic::{AtomicBool, Ordering};

static ASLEEP: AtomicBool = AtomicBool::new(false);
static SCREENS_ASLEEP: AtomicBool = AtomicBool::new(false);
static LOCKED: AtomicBool = AtomicBool::new(false);
static SWITCHED_OUT: AtomicBool = AtomicBool::new(false);

/// Whether nobody is at the Mac right now.
///
/// Starts out present: the app launches at login or by hand, and either way
/// someone is there.
pub fn is_away() -> bool {
    [&ASLEEP, &SCREENS_ASLEEP, &LOCKED, &SWITCHED_OUT]
        .into_iter()
        .any(|flag| flag.load(Ordering::Relaxed))
}

/// Starts listening, once, at startup. `on_change` runs whenever any of the
/// four changes, so the scheduler can act on a return straight away rather
/// than up to a tick later.
///
/// Must be called on the main thread: the notifications are delivered there.
#[cfg(target_os = "macos")]
pub fn install(on_change: impl Fn() + Send + Sync + 'static) {
    use std::ptr::NonNull;
    use std::sync::Arc;

    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
        NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
        NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::{
        NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSString,
    };

    let on_change: Arc<dyn Fn() + Send + Sync> = Arc::new(on_change);
    let observe = |center: &NSNotificationCenter,
                   name: &NSString,
                   flag: &'static AtomicBool,
                   away: bool| {
        let on_change = on_change.clone();
        let block = RcBlock::new(move |_note: NonNull<NSNotification>| {
            flag.store(away, Ordering::Relaxed);
            on_change();
        });
        // Safety: no object filter, no queue (delivery on the posting
        // thread, which is the main one), and the block only touches an
        // atomic and a Send + Sync callback.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        // The centre keeps observing for as long as the token lives, and this
        // is meant to last as long as the app does.
        std::mem::forget(token);
    };

    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    // Safety: these are AppKit's own constants, valid for the process.
    unsafe {
        observe(&workspace, NSWorkspaceWillSleepNotification, &ASLEEP, true);
        observe(&workspace, NSWorkspaceDidWakeNotification, &ASLEEP, false);
        observe(&workspace, NSWorkspaceScreensDidSleepNotification, &SCREENS_ASLEEP, true);
        observe(&workspace, NSWorkspaceScreensDidWakeNotification, &SCREENS_ASLEEP, false);
        observe(&workspace, NSWorkspaceSessionDidResignActiveNotification, &SWITCHED_OUT, true);
        observe(&workspace, NSWorkspaceSessionDidBecomeActiveNotification, &SWITCHED_OUT, false);
    }

    // Locking has no AppKit notification of its own; these distributed ones
    // are what the system posts, and what every lock-aware app listens for.
    let distributed = NSDistributedNotificationCenter::defaultCenter();
    observe(&distributed, &NSString::from_str("com.apple.screenIsLocked"), &LOCKED, true);
    observe(&distributed, &NSString::from_str("com.apple.screenIsUnlocked"), &LOCKED, false);
}

#[cfg(not(target_os = "macos"))]
pub fn install(_on_change: impl Fn() + Send + Sync + 'static) {}
