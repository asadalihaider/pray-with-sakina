import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type PrayerKey = "fajr" | "zuhr" | "asr" | "maghrib" | "isha";

type QazaRow = {
  prayer: PrayerKey;
  label: string;
  remaining: number;
  madeUp: number;
  backlog: number;
};

type QazaView = { total: number; madeUp: number; rows: QazaRow[] };

/// Each prayer is measured against its own debt, not against the others: a
/// bar that only compared rows said how Isha stood beside Fajr, never how
/// far Isha had come.
function progress(row: QazaRow) {
  const owed = row.madeUp + row.remaining;
  return owed === 0 ? 0 : (row.madeUp / owed) * 100;
}

/// Thousands separated: a backlog of years runs to five figures, and
/// "30120" has to be counted where "30,120" can be read.
const count = (n: number) => n.toLocaleString();

function summary(row: QazaRow) {
  if (row.madeUp + row.remaining === 0) return "nothing owed";
  if (row.remaining === 0) return `${count(row.madeUp)} made up · done`;
  return `${count(row.madeUp)} made up · ${count(row.remaining)} left`;
}

/// The screen's heading, said as a sentence rather than shown as figures.
/// Two large numbers beside a title stopped fitting once a backlog ran to
/// years; a sentence has room, and reads as encouragement rather than a
/// scoreboard.
function Progress({ view }: { view: QazaView }) {
  if (view.madeUp === 0) {
    return <>Log your missed prayers as you make them up</>;
  }
  return (
    <>
      You've made up{" "}
      <span className="is-made-up">
        {count(view.madeUp)} {view.madeUp === 1 ? "prayer" : "prayers"}
      </span>{" "}
      and <span className="is-left">{count(view.total)}</span>{" "}
      {view.total === 1 ? "is" : "are"} still remaining
    </>
  );
}

export default function Qaza() {
  const [view, setView] = useState<QazaView | null>(null);
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    invoke<QazaView>("get_qaza").then(setView).catch(() => setView(null));
  }, []);

  if (!view) return <div className="tab-body" />;

  const madeUp = async (prayer: PrayerKey) =>
    setView(await invoke<QazaView>("add_madeup", { prayer }));

  const setBacklog = async (prayer: PrayerKey, raw: string) => {
    const count = Number.parseInt(raw, 10);
    if (Number.isNaN(count)) return;
    setView(await invoke<QazaView>("set_backlog", { prayer, count }));
  };

  if (view.total === 0 && !editing) {
    return (
      <div className="tab-body">
        <div className="qaza-empty">
          No Qaza remaining.
          <br />
          <span>Alhamdulillah.</span>
          {view.madeUp > 0 && (
            <span className="qaza-empty-made-up">{count(view.madeUp)} made up</span>
          )}
        </div>
        <button className="qaza-link" onClick={() => setEditing(true)}>
          Add older Qaza backlog
        </button>
      </div>
    );
  }

  return (
    <div className="tab-body">
      <div className="qaza-head">
        <div className="qaza-title">Qaza</div>
        <div className="qaza-summary">
          <Progress view={view} />
        </div>
      </div>

      <ul className="qaza-list">
        {view.rows.map((row) => (
          <li key={row.prayer} className="qaza-row">
            <div className="qaza-line">
              <div className="qaza-name">{row.label}</div>
              {editing ? (
                <input
                  className="qaza-input"
                  type="number"
                  min={0}
                  defaultValue={row.backlog}
                  onBlur={(event) => setBacklog(row.prayer, event.target.value)}
                />
              ) : (
                <>
                  <div className="qaza-count">{summary(row)}</div>
                  <button
                    className="qaza-madeup"
                    disabled={row.remaining === 0}
                    onClick={() => madeUp(row.prayer)}
                    title="Log a made-up prayer"
                    aria-label={`Log a made-up ${row.label} prayer`}
                  >
                    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
                      <path
                        d="M5 1v8M1 5h8"
                        stroke="currentColor"
                        strokeWidth="1.5"
                        strokeLinecap="round"
                      />
                    </svg>
                  </button>
                </>
              )}
            </div>
            {/* Green is only ever progress. What is still owed is grey, not
                red: red means missed on time, and a prayer not yet made up
                is work still to do rather than a second failure. */}
            {!editing && (
              <div className="qaza-bar">
                <div className="qaza-fill" style={{ width: `${progress(row)}%` }} />
              </div>
            )}
          </li>
        ))}
      </ul>

      <button className="qaza-link" onClick={() => setEditing((value) => !value)}>
        {editing ? "Done" : "Add older Qaza backlog"}
      </button>
    </div>
  );
}
