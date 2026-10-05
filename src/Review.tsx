import { invoke } from "@tauri-apps/api/core";

export type ReviewItem = {
  date: string;
  prayer: "fajr" | "zuhr" | "asr" | "maghrib" | "isha";
  label: string;
  arabic: string;
  day: string;
  time: string;
};

/// A summary, not a list. Logging happens on the month grid, which already
/// does it and does not turn into a wall of rows when a backlog builds up.
export default function Review({
  items,
  onLater,
  onLogNow,
}: {
  items: ReviewItem[];
  onLater: () => void;
  onLogNow: () => void;
}) {
  const days = new Set(items.map((item) => item.date)).size;

  const finish = async (next: () => void) => {
    await invoke("finish_review");
    next();
  };

  return (
    <div className="tab-body review">
      <div className="review-body">
        <div className="review-count">{items.length}</div>
        <div className="review-headline">
          {items.length === 1 ? "prayer unlogged" : "prayers unlogged"}
        </div>
        <div className="review-detail">
          {days === 1
            ? "From yesterday."
            : `Across the last ${days} days.`}{" "}
          Nothing is counted as Qaza unless you mark it missed.
        </div>
      </div>

      <div className="review-actions">
        <button className="review-later" onClick={() => finish(onLater)}>
          Review later
        </button>
        <button className="review-now" onClick={() => finish(onLogNow)}>
          Log them
        </button>
      </div>
    </div>
  );
}
