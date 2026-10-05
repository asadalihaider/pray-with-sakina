import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type PrayerKey = "fajr" | "zuhr" | "asr" | "maghrib" | "isha";

type QazaRow = {
  prayer: PrayerKey;
  label: string;
  remaining: number;
  backlog: number;
  share: number;
};

type QazaView = { total: number; rows: QazaRow[] };

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
        <div className="qaza-total">{view.total}</div>
        <div className="qaza-total-label">prayers remaining</div>
      </div>

      <ul className="qaza-list">
        {view.rows.map((row) => (
          <li key={row.prayer} className="qaza-row">
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
                <div className="qaza-bar">
                  <div
                    className="qaza-fill"
                    style={{
                      width: `${Math.max(row.share * 100, row.remaining > 0 ? 4 : 0)}%`,
                      background: `var(--${row.prayer})`,
                    }}
                  />
                </div>
                <div className="qaza-count">{row.remaining}</div>
                <button
                  className="qaza-madeup"
                  disabled={row.remaining === 0}
                  onClick={() => madeUp(row.prayer)}
                  title="Log a made-up prayer"
                >
                  +
                </button>
              </>
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
