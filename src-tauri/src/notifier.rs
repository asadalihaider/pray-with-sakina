//! Reminders through `UNUserNotificationCenter`.
//!
//! The app used to deliver through `NSUserNotification`, which Apple
//! deprecated. On current macOS that API still *delivers* — notifications
//! turn up in Notification Center — but they are never presented on screen,
//! and the app never appears in System Settings → Notifications, so there is
//! no alert style for the user to choose. A reminder nobody sees is not a
//! reminder.
//!
//! This is the supported path. It asks permission once, it registers the app
//! where the user can configure it, and it presents.
//!
//! It has one hard requirement: **the app must carry a real code signature.**
//! Ad-hoc signing is not enough — `requestAuthorization` returns denied
//! without ever showing a prompt, the status stays `NotDetermined`, and
//! delivery fails with `UNErrorDomain` 1, "notifications not allowed". Any
//! ordinary signature satisfies it, including a self-signed one; it does not
//! have to come from a paid Apple Developer account.
//!
//! Nothing calls this yet. It is kept because it is the only path that can
//! put a reminder on screen, and because proving that took a while.
#![allow(dead_code)]

use std::sync::mpsc;
use std::time::Duration;

use block2::{DynBlock, StackBlock};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{define_class, msg_send, AnyThread};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAlertStyle, UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
    UNNotification,
    UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
    UNNotificationCategoryOptions, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNNotificationSetting, UNNotificationSettings,
    UNUserNotificationCenter,
    UNUserNotificationCenterDelegate,
};

/// How long to wait for one of AppKit's completion handlers.
///
/// Every call here is asynchronous and answers on an arbitrary queue. The
/// callers are a Tauri command and a background scheduler thread, both of
/// which want an answer rather than a callback, so each call is bridged to a
/// channel. The timeout exists because a handler that never fires must not
/// hang a reminder forever.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);

fn center() -> Retained<UNUserNotificationCenter> {
    UNUserNotificationCenter::currentNotificationCenter()
}

/// What macOS currently permits, without asking for anything.
pub fn status() -> Permission {
    let (sender, receiver) = mpsc::channel();
    let handler = StackBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
        // Safety: the handler owns this reference for the duration of the
        // call, which is inside this block.
        let settings = unsafe { settings.as_ref() };
        let _ = sender.send(settings.authorizationStatus());
    });
    center().getNotificationSettingsWithCompletionHandler(&handler);

    match receiver.recv_timeout(REPLY_TIMEOUT) {
        Ok(status) => Permission::from(status),
        Err(_) => Permission::Unknown,
    }
}

/// What macOS will actually do with a reminder, in its own words.
///
/// Worth asking rather than inferring. Whether a reminder appears on screen
/// is the user's setting, and the only honest way to tell them what it is
/// set to is to read it — the alternative is guessing from whether they
/// noticed something.
pub fn presentation() -> Presentation {
    let (sender, receiver) = mpsc::channel();
    let handler = StackBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
        // Safety: the handler owns this reference for the duration of the
        // call, which is inside this block.
        let settings = unsafe { settings.as_ref() };
        let _ = sender.send(Presentation {
            permission: Permission::from(settings.authorizationStatus()),
            style: match settings.alertStyle() {
                UNAlertStyle::None => Style::Off,
                UNAlertStyle::Banner => Style::Temporary,
                UNAlertStyle::Alert => Style::Persistent,
                _ => Style::Unknown,
            },
            on_screen: settings.alertSetting() == UNNotificationSetting::Enabled,
            in_centre: settings.notificationCenterSetting() == UNNotificationSetting::Enabled,
        });
    });
    center().getNotificationSettingsWithCompletionHandler(&handler);

    receiver.recv_timeout(REPLY_TIMEOUT).unwrap_or_default()
}

/// Asks for permission, which macOS turns into a prompt the first time and
/// answers from its own records afterwards.
pub fn request() -> Permission {
    let (sender, receiver) = mpsc::channel();
    let handler = StackBlock::new(move |granted: objc2::runtime::Bool, _error: *mut NSError| {
        let _ = sender.send(granted.as_bool());
    });
    center().requestAuthorizationWithOptions_completionHandler(
        // Alert and sound. Not badge: a menu bar app has no Dock tile to
        // badge, and asking for what we cannot use invites a refusal.
        UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
        &handler,
    );

    match receiver.recv_timeout(REPLY_TIMEOUT) {
        Ok(true) => Permission::Granted,
        Ok(false) => Permission::Denied,
        Err(_) => Permission::Unknown,
    }
}

/// Posts a notification with no button — a test reminder, which must never
/// be able to record a prayer that was not prayed.
pub fn deliver(title: &str, body: &str) -> Result<(), String> {
    post(&format!("test:{}", uuid::Uuid::new_v4()), title, body, None)
}

fn post(
    identifier: &str,
    title: &str,
    body: &str,
    category: Option<&str>,
) -> Result<(), String> {
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    if let Some(category) = category {
        content.setCategoryIdentifier(&NSString::from_str(category));
    }

    let identifier = NSString::from_str(identifier);
    // A nil trigger means deliver immediately rather than schedule.
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &identifier,
        &content,
        None,
    );

    let (sender, receiver) = mpsc::channel();
    let handler = StackBlock::new(move |error: *mut NSError| {
        let message = if error.is_null() {
            None
        } else {
            // Safety: non-null for the duration of this handler.
            Some(unsafe { &*error }.localizedDescription().to_string())
        };
        let _ = sender.send(message);
    });
    center().addNotificationRequest_withCompletionHandler(&request, Some(&handler));

    match receiver.recv_timeout(REPLY_TIMEOUT) {
        Ok(None) => Ok(()),
        Ok(Some(message)) => Err(message),
        Err(_) => Err("macOS did not answer the request to show a reminder.".into()),
    }
}

/// How long a reminder stays on screen, as macOS has it configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Style {
    /// Nothing appears on screen at all; reminders only reach Notification
    /// Center, which is the one setting that defeats the whole app.
    Off,
    /// macOS's default: it dismisses itself after a few seconds.
    Temporary,
    /// Stays until the user deals with it. What this app wants.
    Persistent,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Presentation {
    pub permission: Permission,
    pub style: Style,
    /// Whether anything is allowed on screen, separately from its style.
    pub on_screen: bool,
    pub in_centre: bool,
}

/// macOS's answer, reduced to what the UI has to say about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    /// Never asked. The prompt has not been shown yet.
    #[default]
    Unasked,
    Granted,
    Denied,
    /// Asked and got no answer, which is not the same as a refusal and must
    /// not be presented as one.
    Unknown,
}

impl From<UNAuthorizationStatus> for Permission {
    fn from(status: UNAuthorizationStatus) -> Self {
        match status {
            UNAuthorizationStatus::NotDetermined => Permission::Unasked,
            UNAuthorizationStatus::Denied => Permission::Denied,
            UNAuthorizationStatus::Authorized
            | UNAuthorizationStatus::Provisional
            | UNAuthorizationStatus::Ephemeral => Permission::Granted,
            _ => Permission::Unknown,
        }
    }
}

/// The action identifier for the button on a reminder.
const PRAYED_ACTION: &str = "sakina.prayed";
/// The category the button is attached to. A notification without this
/// category gets no button, which is what a test reminder wants.
const REMINDER_CATEGORY: &str = "sakina.reminder";

/// What the user did with a reminder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Tapped the button on the notification. Carries the prayer the
    /// reminder was for, read back out of the request identifier.
    Prayed(String),
    /// Clicked the notification itself, which should open the panel.
    Opened,
}

type Handler = Box<dyn Fn(Act) + Send + Sync>;
static ON_ACT: std::sync::OnceLock<Handler> = std::sync::OnceLock::new();

define_class!(
    /// Without a delegate, macOS will not present a notification while the
    /// app that sent it is frontmost, and nothing comes back when the user
    /// acts on one. Both of those matter here: the test reminder is sent
    /// from an open panel, and the whole point of the button is to log a
    /// prayer without opening the app.
    #[unsafe(super(NSObject))]
    #[thread_kind = AnyThread]
    #[name = "SakinaNotificationDelegate"]
    struct Delegate;

    impl Delegate {}

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Show it on screen and keep it in Notification Center. Saying
            // so explicitly is what overrides the default of staying quiet
            // while we are the frontmost app.
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &DynBlock<dyn Fn()>,
        ) {
            let action = response.actionIdentifier().to_string();
            let identifier = response
                .notification()
                .request()
                .identifier()
                .to_string();

            if let Some(handler) = ON_ACT.get() {
                if action == PRAYED_ACTION {
                    if let Some(prayer) = prayer_from(&identifier) {
                        handler(Act::Prayed(prayer));
                    }
                } else {
                    // Any other identifier is the default action, which is
                    // the user clicking the notification body.
                    handler(Act::Opened);
                }
            }
            completion.call(());
        }
    }
);

/// Reminders carry the prayer in their identifier, so the response does not
/// need a side table to know what was being reminded about.
fn prayer_from(identifier: &str) -> Option<String> {
    identifier
        .strip_prefix("prayer:")
        .and_then(|rest| rest.split(':').next())
        .map(str::to_string)
}

/// One identifier per prayer, deliberately stable.
///
/// A request whose identifier matches one already delivered replaces it
/// rather than arriving beside it. Nudges for the same prayer therefore
/// update the notification that is already waiting, instead of stacking:
/// leave the Mac for an afternoon and you come back to one unanswered
/// question about Zuhr, not ten. Dismiss it and the next nudge posts
/// afresh, which is the right way to ask again.
fn reminder_identifier(prayer: &str) -> String {
    format!("prayer:{prayer}")
}

/// Registers the delegate and the button, once, at startup.
pub fn install(on_act: impl Fn(Act) + Send + Sync + 'static) {
    let _ = ON_ACT.set(Box::new(on_act));

    let delegate: Retained<Delegate> = unsafe { msg_send![Delegate::alloc(), init] };
    center().setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    // The notification centre holds the delegate weakly, so letting this
    // drop would leave it pointing at freed memory the first time a
    // notification arrived.
    std::mem::forget(delegate);

    let prayed = UNNotificationAction::actionWithIdentifier_title_options(
        &NSString::from_str(PRAYED_ACTION),
        &NSString::from_str("Prayed"),
        UNNotificationActionOptions::empty(),
    );
    let category =
        UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &NSString::from_str(REMINDER_CATEGORY),
            &NSArray::from_retained_slice(&[prayed]),
            &NSArray::from_retained_slice(&[]),
            UNNotificationCategoryOptions::empty(),
        );
    center().setNotificationCategories(&NSSet::from_retained_slice(&[category]));
}

/// Posts a reminder for a prayer, with the button on it.
pub fn deliver_reminder(prayer: &str, title: &str, body: &str) -> Result<(), String> {
    post(&reminder_identifier(prayer), title, body, Some(REMINDER_CATEGORY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_nudge_for_a_prayer_carries_the_same_identifier() {
        // The whole point: macOS replaces a delivered notification whose
        // identifier matches, so a second nudge must not look like a
        // different notification or it arrives beside the first.
        assert_eq!(reminder_identifier("zuhr"), reminder_identifier("zuhr"));
        assert_ne!(reminder_identifier("zuhr"), reminder_identifier("asr"));
    }

    #[test]
    fn the_prayer_survives_the_round_trip() {
        // The response carries no payload of its own, so the identifier is
        // the only place the prayer can be read back from.
        for prayer in ["fajr", "zuhr", "asr", "maghrib", "isha"] {
            assert_eq!(
                prayer_from(&reminder_identifier(prayer)).as_deref(),
                Some(prayer)
            );
        }
    }

    #[test]
    fn a_test_notification_is_not_mistaken_for_a_reminder() {
        assert_eq!(prayer_from("test:anything"), None);
    }
}
