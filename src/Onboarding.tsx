import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import CitySearch, { type Place } from "./CitySearch";
import type { SettingsData } from "./Settings";

/// What macOS says it will do with a reminder. Read rather than assumed:
/// whether one appears on screen is the user's setting, and the only honest
/// way to say what it is set to is to ask.
type Reminders = {
  permission: "unasked" | "granted" | "denied" | "unknown";
  style: "off" | "temporary" | "persistent" | "unknown";
  onScreen: boolean;
  inCentre: boolean;
};

type PrayerKey = "fajr" | "zuhr" | "asr" | "maghrib" | "isha";
const PRAYERS: PrayerKey[] = ["fajr", "zuhr", "asr", "maghrib", "isha"];

const METHODS: [string, string][] = [
  ["karachi", "Karachi"],
  ["muslim_world_league", "Muslim World League"],
  ["egyptian", "Egyptian"],
  ["umm_al_qura", "Umm al-Qura"],
  ["north_america", "ISNA"],
  ["dubai", "Dubai"],
  ["kuwait", "Kuwait"],
  ["qatar", "Qatar"],
  ["singapore", "Singapore"],
  ["tehran", "Tehran"],
  ["turkey", "Turkey"],
];

function label(prayer: PrayerKey) {
  return prayer[0].toUpperCase() + prayer.slice(1);
}

export default function Onboarding({
  settings,
  onDone,
}: {
  settings: SettingsData;
  onDone: () => void;
}) {
  const [step, setStep] = useState(0);
  const [draft, setDraft] = useState(settings);
  const [backlog, setBacklog] = useState<Record<string, string>>({});
  const [locating, setLocating] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const [reminders, setReminders] = useState<Reminders | null>(null);

  // Everything here has a working default, so leaving at any point has to
  // be a complete exit rather than a half-configured state.
  const finish = async () => {
    await invoke("save_settings", {
      settings: { ...draft, onboarded: true },
    });
    for (const prayer of PRAYERS) {
      const count = Number.parseInt(backlog[prayer] ?? "", 10);
      if (!Number.isNaN(count) && count > 0) {
        await invoke("set_backlog", { prayer, count });
      }
    }
    onDone();
  };

  // Sending on arrival rather than on a button press. The screen's whole
  // job is to let the user *see* a reminder, and a button they have to find
  // first is a screen that explains instead of demonstrating. It is also the
  // natural moment for macOS to ask permission.
  const announced = useRef(false);
  useEffect(() => {
    if (step !== 2) return;
    const look = () =>
      invoke<Reminders>("reminder_permission").then(setReminders).catch(() => {});
    if (!announced.current) {
      announced.current = true;
      invoke("send_test_reminder")
        .catch((error) => setSendError(String(error)))
        .finally(look);
    }
    // Looked at again when the panel comes back, because the way out of
    // this screen is System Settings and the answer changes while the user
    // is over there.
    window.addEventListener("focus", look);
    look();
    return () => window.removeEventListener("focus", look);
  }, [step]);

  // Everything this screen asks for is done: permission given, and
  // reminders set to wait on screen rather than slide away.
  const remindersReady =
    reminders?.permission === "granted" && reminders?.style === "persistent";

  const choose = (place: Place) =>
    setDraft({
      ...draft,
      locationName: place.name,
      latitude: place.latitude,
      longitude: place.longitude,
      timezone: place.timezone,
    });

  return (
    <div className="tab-body onboarding">
      <div className="onboard-dots">
        {[0, 1, 2, 3].map((index) => (
          <span key={index} className={index === step ? "is-active" : ""} />
        ))}
      </div>

      {step === 0 && (
        <div className="onboard-step">
          <div className="onboard-title">Where are you?</div>
          <div className="onboard-note">
            Prayer times are calculated from your location. Nothing is sent
            anywhere except the city name you search for.
          </div>
          <CitySearch onPick={choose} onBusy={setLocating} />
          <div className="onboard-current">{draft.locationName}</div>
        </div>
      )}

      {step === 1 && (
        <div className="onboard-step">
          <div className="onboard-title">How should times be worked out?</div>
          <div className="onboard-note">
            Match whatever your masjid uses. You can fine-tune each prayer by
            a few minutes later.
          </div>
          <div className="row">
            <span className="row-label">Method</span>
            <select
              value={draft.method}
              onChange={(event) =>
                setDraft({ ...draft, method: event.target.value })
              }
            >
              {METHODS.map(([value, name]) => (
                <option key={value} value={value}>
                  {name}
                </option>
              ))}
            </select>
          </div>
          <div className="row">
            <span className="row-label">Asr madhab</span>
            <select
              value={draft.madhab}
              onChange={(event) =>
                setDraft({
                  ...draft,
                  madhab: event.target.value as "hanafi" | "shafi",
                })
              }
            >
              <option value="hanafi">Hanafi</option>
              <option value="shafi">Shafi</option>
            </select>
          </div>
        </div>
      )}

      {step === 2 && (
        <div className="onboard-step">
          <div className="onboard-title">Notifications that stay</div>
          <div className="onboard-note">
            One has just been sent, so you can see what a reminder looks
            like. Sakina is only useful if a reminder{" "}
            <strong>waits</strong> for you — one that disappears on its own
            is a prayer missed for the very reason you installed this.
          </div>
          {/* Each case gets the one instruction that applies to it. A
              screen that lists every possibility is a screen the user has
              to diagnose themselves. */}
          {reminders?.permission === "denied" ? (
            <div className="onboard-note onboard-aside is-error">
              macOS is blocking Sakina's notifications, so nothing will
              reach you. Open your Mac settings below and turn{" "}
              <strong>Allow notifications</strong> on. Once it has been
              refused, macOS will not ask again on its own.
            </div>
          ) : reminders?.style === "off" || reminders?.onScreen === false ? (
            <div className="onboard-note onboard-aside is-error">
              Reminders are reaching Notification Center but never your
              screen. Open your Mac settings below and set the alert style
              to <strong>Persistent</strong>.
            </div>
          ) : reminders?.style === "persistent" ? (
            <div className="onboard-note onboard-aside">
              Set to <strong>Persistent</strong> — a reminder will wait on
              screen until you dismiss it. Nothing else to do here.
            </div>
          ) : sendError ? (
            <div className="onboard-note onboard-aside is-error">
              {sendError}
            </div>
          ) : (
            <div className="onboard-note onboard-aside">
              Open your Mac settings below, then set Sakina's alert style to{" "}
              <strong>Persistent</strong>. Reminders will then stay on screen
              until you dismiss them yourself.
            </div>
          )}
          <button
            className="onboard-settings"
            onClick={() => invoke("open_notification_settings")}
          >
            Open your Mac settings
          </button>
        </div>
      )}

      {step === 3 && (
        <div className="onboard-step">
          <div className="onboard-title">Any Qaza to carry over?</div>
          <div className="onboard-note">
            A rough figure is fine, and you can leave this empty. Only
            prayers you mark as missed are added from here on.
          </div>
          {PRAYERS.map((prayer) => (
            <div className="row" key={prayer}>
              <span className="row-label">{label(prayer)}</span>
              <input
                className="minutes"
                type="number"
                min={0}
                placeholder="0"
                value={backlog[prayer] ?? ""}
                onChange={(event) =>
                  setBacklog({ ...backlog, [prayer]: event.target.value })
                }
              />
            </div>
          ))}
        </div>
      )}

      <div className="onboard-actions">
        <button className="onboard-skip" onClick={finish} disabled={locating}>
          Skip
        </button>
        {step < 3 ? (
          <button
            className="onboard-next"
            onClick={() => setStep(step + 1)}
            disabled={locating}
          >
            {/* On the notifications screen the forward button is a
                refusal rather than a confirmation — unless macOS says it
                is already set up, in which case calling it "set up later"
                is simply untrue. */}
            {step === 2 && !remindersReady ? "Set up later" : "Next"}
          </button>
        ) : (
          <button className="onboard-next" onClick={finish}>
            Done
          </button>
        )}
      </div>
    </div>
  );
}
