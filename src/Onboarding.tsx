import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import CitySearch, { type Place } from "./CitySearch";
import type { SettingsData } from "./Settings";

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
        {[0, 1, 2].map((index) => (
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
          <CitySearch onPick={choose} />
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
        <button className="onboard-skip" onClick={finish}>
          Skip
        </button>
        {step < 2 ? (
          <button className="onboard-next" onClick={() => setStep(step + 1)}>
            Next
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
