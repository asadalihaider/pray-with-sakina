import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { formatClock, formatDuration } from "./format";
import Review, { type ReviewItem } from "./Review";
import Stats from "./Stats";
import Qaza from "./Qaza";
import Settings, { type SettingsData } from "./Settings";
import Onboarding from "./Onboarding";
import CitySearch, { type Place } from "./CitySearch";
import "./App.css";

type PrayerKey = "fajr" | "zuhr" | "asr" | "maghrib" | "isha";
type RowStatus = "prayed" | "missed" | "unlogged" | "upcoming" | "active" | "none";

type Focus = {
  prayer: PrayerKey;
  label: string;
  arabic: string;
  active: boolean;
  remainingMs: number;
  windowFraction: number;
  startsAt: string;
  endsAt: string;
  prayedAt: string | null;
};

type Row = {
  key: PrayerKey | "sunrise";
  label: string;
  arabic: string;
  time: string;
  status: RowStatus;
  prayedAt: string | null;
};

type TodayView = {
  now: string;
  date: string;
  timezone: string;
  footer: string;
  focus: Focus;
  rows: Row[];
  mulk: string | null;
};

type Tab = "today" | "stats" | "qaza" | "settings";

const REFRESH_MS = 5000;
const RING_RADIUS = 62;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

function TabIcon({ tab }: { tab: Tab }) {
  const common = {
    width: 18,
    height: 18,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.7,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };
  switch (tab) {
    case "today":
      return (
        <svg {...common}>
          <circle cx="12" cy="12" r="9" />
          <path d="M12 7.5V12l3 1.8" />
        </svg>
      );
    case "stats":
      return (
        <svg {...common}>
          <path d="M5 19V11M12 19V5M19 19v-5" />
        </svg>
      );
    case "qaza":
      return (
        <svg {...common}>
          <path d="M4 7h16M4 12h11M4 17h7" />
        </svg>
      );
    case "settings":
      return (
        <svg {...common}>
          <path d="M5 8h14M5 16h14" />
          <circle cx="10" cy="8" r="2.2" />
          <circle cx="15" cy="16" r="2.2" />
        </svg>
      );
  }
}

function Ring({ view }: { view: TodayView }) {
  const { focus } = view;
  const prayed = focus.prayedAt !== null;

  return (
    <div className="ring-block">
      <div className="ring">
        <svg width="144" height="144">
          <circle className="ring-track" cx="72" cy="72" r={RING_RADIUS} />
          <circle
            className="ring-arc"
            cx="72"
            cy="72"
            r={RING_RADIUS}
            strokeDasharray={RING_CIRCUMFERENCE}
            strokeDashoffset={RING_CIRCUMFERENCE * (1 - focus.windowFraction)}
          />
        </svg>
        <div className="ring-center">
          <span className="ring-prayer">
            {focus.label}
            {prayed && <span className="ring-tick"> ✓</span>}
          </span>
          <span className="ring-remaining">
            {formatDuration(focus.remainingMs)}
          </span>
        </div>
      </div>
    </div>
  );
}

/// A daily habit rather than a prayer, so it sits under the list with a
/// rule above it and no time of its own — and it is logged by tapping the
/// row, since there is no window for a button to belong to.
function MulkRow({
  view,
  onToggle,
}: {
  view: TodayView;
  onToggle: (recited: boolean) => void;
}) {
  if (view.mulk === null) return null;
  const recited = view.mulk === "prayed";

  return (
    <button
      className="prayer-row mulk-row"
      onClick={() => onToggle(!recited)}
      title={recited ? "Mark as not recited" : "Mark as recited"}
    >
      <span className={`chip chip-${view.mulk}`} />
      <span className="prayer-name">Surah Mulk</span>
      <span className="prayer-arabic">الملك</span>
      <span className={`mulk-state${recited ? " is-done" : ""}`}>
        {recited ? "Recited" : "Tap to log"}
      </span>
    </button>
  );
}

function PrayerList({ view }: { view: TodayView }) {
  return (
    <ul className="prayer-list">
      {view.rows.map((row) => (
        <li
          key={row.key}
          className={[
            "prayer-row",
            row.key === "sunrise" ? "is-sunrise" : "",
            row.key === view.focus.prayer ? "is-focus" : "",
          ]
            .filter(Boolean)
            .join(" ")}
        >
          <span className={`chip chip-${row.status === "upcoming" ? "empty" : row.status}`} />
          <span className="prayer-name">{row.label}</span>
          <span className="prayer-arabic">{row.arabic}</span>
          <span className="prayer-time">{formatClock(row.time, view.timezone)}</span>
        </li>
      ))}
    </ul>
  );
}

export default function App() {
  const [view, setView] = useState<TodayView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tab, setTab] = useState<Tab>("today");
  const [review, setReview] = useState<ReviewItem[] | null>(null);
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [changingPlace, setChangingPlace] = useState(false);

  // The web view follows the system by default; an explicit choice overrides
  // it through a data attribute the stylesheet keys off.
  const applyTheme = useCallback(async () => {
    try {
      const current = await invoke<SettingsData>("get_settings");
      setSettings(current);
      const root = document.documentElement;
      if (current.theme === "system") {
        delete root.dataset.theme;
      } else {
        root.dataset.theme = current.theme;
      }
    } catch {
      /* styling falls back to the system preference */
    }
  }, []);

  useEffect(() => {
    applyTheme();
  }, [applyTheme]);

  const refresh = useCallback(async () => {
    try {
      setView(await invoke<TodayView>("get_today"));
      setError(null);
    } catch (problem) {
      setError(String(problem));
    }
  }, []);

  // Yesterday's unanswered prayers get one pass before the day starts.
  const loadReview = useCallback(async () => {
    try {
      const items = await invoke<ReviewItem[]>("get_review");
      setReview(items.length > 0 ? items : null);
    } catch {
      setReview(null);
    }
  }, []);

  useEffect(() => {
    loadReview();
  }, [loadReview]);

  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, REFRESH_MS);
    window.addEventListener("focus", refresh);
    return () => {
      clearInterval(timer);
      window.removeEventListener("focus", refresh);
    };
  }, [refresh]);

  // The web view survives hide/show, so a reopened popover would otherwise
  // show whatever was on screen when it was last dismissed.
  useEffect(() => {
    const pending = listen("popover-shown", refresh);
    return () => {
      pending.then((unlisten) => unlisten());
    };
  }, [refresh]);

  const markPrayed = useCallback(async () => {
    if (!view) return;
    try {
      setView(
        await invoke<TodayView>("log_prayer", {
          prayer: view.focus.prayer,
          status: "prayed",
        })
      );
    } catch (problem) {
      setError(String(problem));
    }
  }, [view]);

  // Location lives on the home screen because it is the one setting tied to
  // what the screen is showing, and burying it made Settings scroll.
  const changePlace = useCallback(
    async (place: Place) => {
      if (!settings) return;
      const next = {
        ...settings,
        locationName: place.name,
        latitude: place.latitude,
        longitude: place.longitude,
        timezone: place.timezone,
      };
      await invoke("save_settings", { settings: next });
      setSettings(next);
      // Deliberately staying put. Closing the panel the instant a place is
      // picked threw the user back to Today before they could see what had
      // been saved — worst with "use my current location", where the whole
      // question is which place it found. The row above updates to the
      // saved name, and the back arrow is right there.
      refresh();
    },
    [settings, refresh]
  );

  const accent = view ? `var(--${view.focus.prayer})` : "var(--zuhr)";

  return (
    <div
      className="popover"
      style={{ ["--accent" as string]: accent }}
      data-tauri-drag-region="deep"
    >
      {error && (
        <div className="error">
          Prayer times are unavailable for this location.
          <br />
          {error}
        </div>
      )}

      {!error && settings && !settings.onboarded && (
        <Onboarding
          settings={settings}
          onDone={() => {
            applyTheme();
            refresh();
            loadReview();
          }}
        />
      )}

      {!error && settings?.onboarded && view && review && (
        <Review
          items={review}
          onLater={() => setReview(null)}
          onLogNow={() => {
            setReview(null);
            setTab("stats");
          }}
        />
      )}

      {!error && settings?.onboarded && view && !review && !changingPlace && tab === "today" && (
        <div className="tab-body">
          <div className="home-head">
            <button
              className="place-chip"
              onClick={() => setChangingPlace(true)}
              title="Change location"
            >
              {settings.locationName}
            </button>
          </div>
          <Ring view={view} />
          {view.focus.prayedAt ? (
            <div className="prayed-note">
              Prayed at {formatClock(view.focus.prayedAt, view.timezone)}
            </div>
          ) : view.focus.active ? (
            <button className="prayed-button" onClick={markPrayed}>
              I've prayed
            </button>
          ) : (
            <div className="prayed-note">
              Starts at {formatClock(view.focus.startsAt, view.timezone)}
            </div>
          )}
          <PrayerList view={view} />
          <MulkRow
            view={view}
            onToggle={async (recited) => {
              await invoke("log_mulk", { date: view.date, recited });
              refresh();
            }}
          />
        </div>
      )}

      {!error && settings?.onboarded && !review && changingPlace && tab === "today" && (
        <div className="tab-body place-panel">
          <div className="panel-head">
            <button className="back-arrow" onClick={() => setChangingPlace(false)}>
              ‹
            </button>
            <span className="panel-title">Location</span>
          </div>
          <div className="row">
            <span className="row-label">{settings.locationName}</span>
            <span className="row-value">{settings.timezone}</span>
          </div>
          <CitySearch onPick={changePlace} />
        </div>
      )}

      {!error && settings?.onboarded && !review && tab === "stats" && <Stats />}
      {!error && settings?.onboarded && !review && tab === "qaza" && <Qaza />}

      {!error && settings?.onboarded && !review && tab === "settings" && (
        <Settings onSaved={() => { refresh(); applyTheme(); }} />
      )}

      {settings?.onboarded && <div className="footer">{view?.footer ?? " "}</div>}

      {settings?.onboarded && (
      <nav className="tabbar">
        {(["today", "stats", "qaza", "settings"] as Tab[]).map((name) => (
          <button
            key={name}
            className={`tab${tab === name ? " is-selected" : ""}`}
            onClick={() => setTab(name)}
            title={name[0].toUpperCase() + name.slice(1)}
          >
            <TabIcon tab={name} />
          </button>
        ))}
      </nav>
      )}
    </div>
  );
}
