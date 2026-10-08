import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type PrayerKey = "fajr" | "zuhr" | "asr" | "maghrib" | "isha";
const PRAYERS: PrayerKey[] = ["fajr", "zuhr", "asr", "maghrib", "isha"];

type FirstNudge =
  | { kind: "offset_minutes"; minutes: number }
  | { kind: "jamaat_time"; hour: number; minute: number };

/// Null until the user picks one. Prayer times cannot be guessed, so
/// nothing is shown before a place is set.
export type StoredPlace = {
  name: string;
  latitude: number;
  longitude: number;
  timezone: string;
};

export type SettingsData = {
  location: StoredPlace | null;
  method: string;
  madhab: "hanafi" | "shafi";
  adjustments: Record<string, number>;
  firstNudge: { prayer: PrayerKey; firstNudge: FirstNudge }[];
  jumuah: FirstNudge | null;
  bedtimeHour: number;
  bedtimeMinute: number;
  reciteMulk: boolean;
  launchAtLogin: boolean;
  theme: "system" | "light" | "dark";
  onboarded: boolean;
};

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

function pad(value: number) {
  return String(value).padStart(2, "0");
}

export default function Settings({ onSaved }: { onSaved: () => void }) {
  const [data, setData] = useState<SettingsData | null>(null);
  // Collapsed by default: most people never touch these, and open they are
  // five rows of zeroes between the method and the reminders.
  const [showShifts, setShowShifts] = useState(false);

  useEffect(() => {
    invoke<SettingsData>("get_settings").then(setData).catch(() => setData(null));
  }, []);

  if (!data) return <div className="tab-body" />;

  const save = async (next: SettingsData) => {
    setData(next);
    await invoke<SettingsData>("save_settings", { settings: next });
    onSaved();
  };

  const nudgeFor = (prayer: PrayerKey) =>
    data.firstNudge.find((entry) => entry.prayer === prayer)?.firstNudge;

  const setNudge = (prayer: PrayerKey, next: FirstNudge) =>
    save({
      ...data,
      firstNudge: data.firstNudge.map((entry) =>
        entry.prayer === prayer ? { ...entry, firstNudge: next } : entry
      ),
    });

  return (
    <div className="tab-body settings">
      <div className="group">
        <div className="group-title">App</div>
        <label className="row">
          <span className="row-label">Track Surah Mulk</span>
          <input
            type="checkbox"
            checked={data.reciteMulk}
            onChange={(event) =>
              save({ ...data, reciteMulk: event.target.checked })
            }
          />
        </label>
        <label className="row">
          <span className="row-label">Launch at login</span>
          <input
            type="checkbox"
            checked={data.launchAtLogin}
            onChange={(event) =>
              save({ ...data, launchAtLogin: event.target.checked })
            }
          />
        </label>
        <div className="row">
          <span className="row-label">Theme</span>
          <select
            value={data.theme}
            onChange={(event) =>
              save({
                ...data,
                theme: event.target.value as "system" | "light" | "dark",
              })
            }
          >
            <option value="system">System</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
        </div>
      </div>
      <div className="group">
        <div className="group-title">Calculation</div>
        <div className="row">
          <span className="row-label">Method</span>
          <select
            value={data.method}
            onChange={(event) => save({ ...data, method: event.target.value })}
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
            value={data.madhab}
            onChange={(event) =>
              save({ ...data, madhab: event.target.value as "hanafi" | "shafi" })
            }
          >
            <option value="hanafi">Hanafi</option>
            <option value="shafi">Shafi</option>
          </select>
        </div>
        <div className="row-note">
          Hanafi puts Asr later in the afternoon than Shafi. It changes Asr
          only.
        </div>
        <button
          className="disclosure"
          onClick={() => setShowShifts((open) => !open)}
        >
          Match my masjid
          <span className="chevron">{showShifts ? "▾" : "▸"}</span>
        </button>
        {showShifts && (
          <>
            <div className="row-note">
              If your masjid's posted times differ from the calculated ones,
              shift each prayer by a few minutes to match.
            </div>
            {PRAYERS.map((prayer) => (
              <div className="row" key={prayer}>
                <span className="row-label">{label(prayer)}</span>
                <input
                  className="minutes"
                  type="number"
                  value={data.adjustments[prayer] ?? 0}
                  onChange={(event) =>
                    save({
                      ...data,
                      adjustments: {
                        ...data.adjustments,
                        [prayer]: Number(event.target.value) || 0,
                      },
                    })
                  }
                />
                <span className="row-unit">min</span>
              </div>
            ))}
          </>
        )}
      </div>

      <div className="group">
        <div className="group-title">Reminders</div>
        {PRAYERS.map((prayer) => {
          const nudge = nudgeFor(prayer);
          if (!nudge) return null;
          const isJamaat = nudge.kind === "jamaat_time";
          return (
            <div className="row" key={prayer}>
              <span className="row-label">{label(prayer)}</span>
              <select
                value={isJamaat ? "jamaat" : "offset"}
                onChange={(event) =>
                  setNudge(
                    prayer,
                    event.target.value === "jamaat"
                      ? { kind: "jamaat_time", hour: 13, minute: 0 }
                      : { kind: "offset_minutes", minutes: 30 }
                  )
                }
              >
                <option value="offset">After start</option>
                <option value="jamaat">Jamaat</option>
              </select>
              {isJamaat ? (
                <input
                  className="minutes"
                  type="time"
                  value={`${pad(nudge.hour)}:${pad(nudge.minute)}`}
                  onChange={(event) => {
                    const [hour, minute] = event.target.value.split(":").map(Number);
                    setNudge(prayer, { kind: "jamaat_time", hour, minute });
                  }}
                />
              ) : (
                <>
                  <input
                    className="minutes"
                    type="number"
                    min={0}
                    value={nudge.minutes}
                    onChange={(event) =>
                      setNudge(prayer, {
                        kind: "offset_minutes",
                        minutes: Number(event.target.value) || 0,
                      })
                    }
                  />
                  <span className="row-unit">min</span>
                </>
              )}
            </div>
          );
        })}
        <div className="row">
          <span className="row-label">Jumu'ah</span>
          <select
            value={data.jumuah ? "jamaat" : "same"}
            onChange={(event) =>
              save({
                ...data,
                jumuah:
                  event.target.value === "jamaat"
                    ? { kind: "jamaat_time", hour: 13, minute: 30 }
                    : null,
              })
            }
          >
            <option value="same">Same as Zuhr</option>
            <option value="jamaat">Jamaat</option>
          </select>
          {data.jumuah && data.jumuah.kind === "jamaat_time" && (
            <input
              className="minutes"
              type="time"
              value={`${pad(data.jumuah.hour)}:${pad(data.jumuah.minute)}`}
              onChange={(event) => {
                const [hour, minute] = event.target.value.split(":").map(Number);
                save({ ...data, jumuah: { kind: "jamaat_time", hour, minute } });
              }}
            />
          )}
        </div>
        <div className="row">
          <span className="row-label">Bedtime</span>
          <input
            className="minutes"
            type="time"
            value={`${pad(data.bedtimeHour)}:${pad(data.bedtimeMinute)}`}
            onChange={(event) => {
              const [hour, minute] = event.target.value.split(":").map(Number);
              save({ ...data, bedtimeHour: hour, bedtimeMinute: minute });
            }}
          />
        </div>
        <div className="row-note">Isha reminders stop at bedtime.</div>
      </div>

      {import.meta.env.DEV && (
        <div className="group">
          <button className="linky" onClick={() => invoke("preview_reminder")}>
            Preview a reminder
          </button>
        </div>
      )}
    </div>
  );
}
