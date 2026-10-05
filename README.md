# Sakina

A prayer companion that lives in the macOS menu bar. It keeps nudging until
you have prayed, and keeps an honest record of what was prayed, missed, or
never answered — without quietly counting anything against you.

Built for people who sit at a laptop all day and lose prayers to a screen,
not to forgetfulness.

## What it does

- **Menu bar countdown** — the current prayer and time left, e.g. `Maghrib · 1h 7m`
- **Reminders** that start at a per-prayer offset and get more frequent as the
  window closes, stopping the moment you log the prayer
- **Prayed / Missed / Unlogged** — an unanswered prayer becomes *Unlogged*,
  never Qaza. Only you can mark something missed.
- **Morning review** — one prompt a day for prayers that closed unanswered
- **Month grid** — a year's habit at a glance, and where past days are corrected
- **Qaza counter** — missed prayers plus an optional backlog, less what you
  have made up
- **Surah Mulk** — an optional daily habit tracked beside the prayers, with its
  own streak
- **No account, no network.** Your records are a file on your Mac and they
  stay there. The app makes no requests of any kind.

## State

macOS works and is in daily use. Phases 1–7 of
[the build plan](prayer-reminder-build-plan.md) are done; phase 8 is living
with it and tuning the nudge timings.

**Windows is not built.** Reminders are macOS-only — the notification path is
an empty stub elsewhere — so the app would run without its main feature. See
the plan's status table.

### Known limitations

| | |
|---|---|
| Reminders are suppressed during **Focus / Do Not Disturb** | macOS only allows an app past Focus with the Time Sensitive entitlement, which needs a signed build and the user opting in. Not something a setting can force. |
| The panel does not appear over a **fullscreen app** | Apple's own SwiftUI `MenuBarExtra` has the same problem, and the documented `presentationOptions` workaround is ignored. |
| An auto-hidden **menu bar** still hides behind the panel | macOS reveals it for the pointer or a real `NSMenu`, and a web view can be neither. |
| Builds are **not notarised** | Notarisation needs a paid Apple Developer account. Without one, macOS refuses a downloaded build on first open: **System Settings → Privacy & Security → Open Anyway**, once per install. |
| The tray icon has **no right-click menu**; Quit is on a **double click** | `tray-icon` 0.24 attaches a menu to the status item permanently, and macOS 27 pops it on left click, swallowing every attempt to open the panel. The menu is built and shown by hand instead. |
| Pausing reminders while the **mic or camera** is in use is a setting that exists but does nothing yet | |

## Running it

Needs [Rust](https://www.rust-lang.org/tools/install) and Node 20+.

```bash
cd app
npm install
npm run tauri dev
```

Reminders will not behave under `tauri dev`: an unbundled binary is not
registered with Launch Services, so macOS misattributes the notification and a
click cannot route back to the app. Test them against a real bundle:

```bash
cd app
npm run tauri build
open src-tauri/target/release/bundle/macos/Sakina.app
```

Tests — 72 of them, covering the prayer time maths, window rules, the nudge
schedule, the storage layer, the review floor, and the settings migrations:

```bash
cd app/src-tauri
cargo test
```

## How it is put together

Tauri 2, with a React + TypeScript front end and the logic in Rust.

```
app/src-tauri/src/
  engine/      prayer times, windows, nudge schedule, prayer state — pure
               functions with no clock of their own, so tests can fix time
  scheduler.rs decides when a reminder is due
  store.rs     SQLite: prayer logs, Qaza entries, Surah Mulk, settings
  settings.rs  settings, and the migrations that got them here
  today.rs     builds what the Today tab shows
  stats.rs     the month grid, streaks and the Qaza ledger
  location.rs  CoreLocation, and naming a place without disclosing it
  icon.rs      the menu bar star, rasterised at runtime
app/src/       the panel: Today, Stats, Qaza, Settings, onboarding
```

**The engine takes `now` as an argument.** Nothing in it reads the clock, so
daylight saving, an Isha window crossing midnight, and waking from sleep after
a window closed are all ordinary test cases rather than things to find in
production.

**The scheduler is a Rust loop, not a web view timer.** A hidden web view can
be throttled by App Nap, which would silently stop reminders. A thread
comparing the clock every 30 seconds also handles sleep for free: the first
tick after waking notices what was missed, and fast-forwards to a single
reminder rather than a burst of the ones it slept through.

**Prayer state is derived, not stored.** A logged answer wins; otherwise the
state falls out of `now` against the window. That is what makes closing a
laptop through a whole window correct without any sleep handling.

### Prayer times

The [`salah`](https://crates.io/crates/salah) crate, defaulting to the Karachi
method with Hanafi Asr. Any of eleven methods, either madhab, and a per-prayer
minute shift to match a masjid's posted times.

The crate panics outright on a few extreme high-latitude, near-solstice
locations — Reykjavik in June, for one — so the engine contains that and
returns an error instead of taking the app down.

### Location and privacy

Two ways to set it, neither of which sends your coordinates anywhere:

- **Search a city** — the text you type goes to Open-Meteo's keyless geocoder,
  which returns coordinates and an IANA timezone. Your position does not.
- **Use my location** — CoreLocation resolves position from nearby Wi-Fi, since
  a Mac has no GPS. The timezone comes from the system clock, and the city name
  from Apple's own geocoder, which already holds the coordinates.

Everything is stored at
`~/Library/Application Support/dev.asadalihaider.sakina/sakina.db`, separate
from the app bundle — rebuilding or reinstalling will not touch your history.
Rows carry `updated_at` and `device_id`. Nothing reads them today; they are
the audit trail if a record ever looks wrong, and they cost a few bytes.
