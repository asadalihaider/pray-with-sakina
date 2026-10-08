import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type PrayerKey = "fajr" | "zuhr" | "asr" | "maghrib" | "isha";
const PRAYERS: PrayerKey[] = ["fajr", "zuhr", "asr", "maghrib", "isha"];

type DayStats = {
  date: string;
  day: number;
  statuses: string[];
  mulk: string | null;
};

type StatsView = {
  label: string;
  year: number;
  month: number;
  days: DayStats[];
  streak: number;
  mulkStreak: number | null;
  onTime: number | null;
  hasNext: boolean;
  today: string;
};

/// `prayer` is null for the Surah Mulk row, which is logged through its own
/// command rather than as a sixth prayer.
type Selection = {
  date: string;
  day: number;
  prayer: PrayerKey | null;
  row: number;
};

function label(prayer: PrayerKey) {
  return prayer[0].toUpperCase() + prayer.slice(1);
}

/// Counts of each state per prayer across the month, which is what turns
/// "I miss prayers" into "I miss Fajr".
function tally(days: DayStats[]) {
  return PRAYERS.map((prayer, row) => {
    const counts = { prayed: 0, missed: 0, unlogged: 0 };
    for (const day of days) {
      const status = day.statuses[row];
      if (status in counts) counts[status as keyof typeof counts] += 1;
    }
    const total = counts.prayed + counts.missed + counts.unlogged;
    return { prayer, ...counts, total };
  });
}

export default function Stats() {
  const [view, setView] = useState<StatsView | null>(null);
  const [offset, setOffset] = useState(0);
  const [selected, setSelected] = useState<Selection | null>(null);
  // The grid disables what cannot be logged, so this only surfaces if the
  // backend refuses something the grid thought was fine.
  const [refused, setRefused] = useState<string | null>(null);

  const load = useCallback(async (monthOffset: number) => {
    const now = new Date();
    const target = new Date(now.getFullYear(), now.getMonth() + monthOffset, 1);
    const next = await invoke<StatsView>("get_stats", {
      year: target.getFullYear(),
      month: target.getMonth() + 1,
    });
    setView(next);
    return next;
  }, []);

  /// Surah Mulk is the row after Isha, not a separate list — walking only
  /// the prayers would step over it into the next day.
  const statusAt = (day: DayStats, row: number) =>
    row < PRAYERS.length ? day.statuses[row] : day.mulk;

  /// The first unlogged cell at or after a point, so clearing a backlog is
  /// one click per entry instead of hunting for the grey squares.
  const nextUnlogged = (
    from: StatsView,
    afterDay: number,
    afterRow: number
  ): Selection | null => {
    const rows =
      from.days[0]?.mulk != null ? PRAYERS.length + 1 : PRAYERS.length;

    for (const day of from.days) {
      for (let row = 0; row < rows; row += 1) {
        const later =
          day.day > afterDay || (day.day === afterDay && row > afterRow);
        if (later && statusAt(day, row) === "unlogged") {
          return {
            date: day.date,
            day: day.day,
            prayer: row < PRAYERS.length ? PRAYERS[row] : null,
            row,
          };
        }
      }
    }
    return null;
  };

  useEffect(() => {
    load(offset).catch(() => setView(null));
  }, [load, offset]);

  const edit = async (status: "prayed" | "missed") => {
    if (!selected) return;
    try {
      if (selected.prayer === null) {
        await invoke("log_mulk", {
          date: selected.date,
          recited: status === "prayed",
        });
      } else {
        await invoke("log_past", {
          date: selected.date,
          prayer: selected.prayer,
          status,
        });
      }
    } catch (problem) {
      setRefused(String(problem));
      return;
    }
    setRefused(null);
    const refreshed = await load(offset);
    setSelected(nextUnlogged(refreshed, selected.day, selected.row));
  };

  if (!view) return <div className="tab-body" />;

  return (
    <div className="tab-body">
      <div className="month-head">
        <button onClick={() => setOffset((value) => value - 1)}>‹</button>
        <span className="month-label">{view.label}</span>
        <button disabled={offset >= 0} onClick={() => setOffset((value) => value + 1)}>
          ›
        </button>
      </div>

        <div className="tiles">
          <div className="tile">
            <div className="tile-value">{view.streak}</div>
            <div className="tile-label">Prayer streak</div>
          </div>
          {view.mulkStreak !== null && (
            <div className="tile">
              <div className="tile-value">{view.mulkStreak}</div>
              <div className="tile-label">Surah streak</div>
            </div>
          )}
          <div className="tile">
            <div className="tile-value">
              {view.onTime === null ? "—" : `${view.onTime}%`}
            </div>
            <div className="tile-label">Prayed</div>
          </div>
        </div>


      <div className="grid-wrap">
        <div
          className="grid"
          style={{
            gridTemplateColumns: `16px repeat(${view.days.length}, 1fr)`,
          }}
        >
          {PRAYERS.map((prayer, row) => (
            <span
              key={prayer}
              className="grid-label"
              style={{ gridColumn: 1, gridRow: row + 1 }}
              title={label(prayer)}
            >
              {prayer[0].toUpperCase()}
            </span>
          ))}
          {view.days[0]?.mulk != null && (
            <span
              className="grid-label"
              style={{ gridColumn: 1, gridRow: PRAYERS.length + 1 }}
              title="Surah Mulk"
            >
              S
            </span>
          )}
          {view.days.map((day) =>
            day.mulk == null ? null : (
              <button
                key={`${day.date}-mulk`}
                className={`cell is-${day.mulk}${
                  selected?.date === day.date && selected.prayer === null
                    ? " is-selected"
                    : ""
                }`}
                style={{ gridColumn: day.day + 1, gridRow: PRAYERS.length + 1 }}
                title={`${day.date} · Surah Mulk`}
                disabled={day.date > view.today}
                onClick={() =>
                  setSelected({
                    date: day.date,
                    day: day.day,
                    prayer: null,
                    row: PRAYERS.length,
                  })
                }
              />
            )
          )}
          {view.days.map((day) =>
            day.statuses.map((status, row) => (
              <button
                key={`${day.date}-${row}`}
                className={`cell is-${status}${
                  selected?.date === day.date && selected.row === row ? " is-selected" : ""
                }`}
                style={{ gridColumn: day.day + 1, gridRow: row + 1 }}
                title={`${day.date} · ${PRAYERS[row]}`}
                disabled={status === "upcoming"}
                onClick={() =>
                  setSelected({
                    date: day.date,
                    day: day.day,
                    prayer: PRAYERS[row],
                    row,
                  })
                }
              />
            ))
          )}
        </div>
      </div>

      <div className="breakdown">
        {tally(view.days).map((row) => (
          <div className="breakdown-row" key={row.prayer}>
            <span className="breakdown-name">{label(row.prayer)}</span>
            <span className="breakdown-bar">
              <span
                style={{
                  width: `${row.total ? (row.prayed / row.total) * 100 : 0}%`,
                  background: "var(--prayed)",
                }}
              />
              <span
                style={{
                  width: `${row.total ? (row.missed / row.total) * 100 : 0}%`,
                  background: "var(--missed)",
                }}
              />
              <span
                style={{
                  width: `${row.total ? (row.unlogged / row.total) * 100 : 0}%`,
                  background: "var(--unlogged-fill)",
                }}
              />
            </span>
            <span className="breakdown-count">
              {row.prayed}/{row.total}
            </span>
          </div>
        ))}
      </div>

      {refused && <div className="row-note">{refused}</div>}

      {selected && (
        <div className="cell-editor">
          <span className="cell-editor-label">
            {selected.prayer === null
              ? "Surah Mulk"
              : label(selected.prayer)}{" "}
            · {selected.day} {view.label.split(" ")[0]}
          </span>
          <button onClick={() => edit("prayed")}>
            {selected.prayer === null ? "Recited" : "Prayed"}
          </button>
          <button onClick={() => edit("missed")}>Missed</button>
          <button className="is-ghost" onClick={() => setSelected(null)}>
            ✕
          </button>
        </div>
      )}
    </div>
  );
}
