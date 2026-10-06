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
- **No account, no sign-in.** Your records are a file on your Mac and they stay
  there. The only request the app ever makes is to look up a city you typed.

## Install

macOS 13 or later, Apple silicon or Intel.

Download `Sakina.zip` from [the latest release][latest], unzip it, and drag
**Sakina.app** into your Applications folder.

The first time you open it, macOS will say it *"could not verify Sakina is free
of malware"*. It is telling the truth: it cannot verify it, because verifying
means paying Apple $99 a year to notarise the build, and this is a weekend
project. The app is signed — just not by someone Apple has been paid to vouch
for.

To open it anyway:

1. Click **Done** on the warning
2. Open **System Settings → Privacy & Security**
3. Scroll to the bottom — there is a line about Sakina being blocked — and
   click **Open Anyway**

Once per install. Updates are not challenged again.

If you would rather skip that, this does the same thing without the warning,
because macOS only flags files that arrive through a browser:

```bash
curl -L https://github.com/asadalihaider/pray-with-sakina/releases/latest/download/Sakina.zip \
  | tar -xf - -C /Applications && open /Applications/Sakina.app
```

### With Homebrew

```bash
brew install --cask asadalihaider/sakina/sakina
```

Homebrew flags what it downloads, so this still goes through **Open Anyway** on
first launch. Adding `--no-quarantine` skips that. The cask will not do it for
you: waiving a security check should be a decision you make rather than one a
file you installed makes quietly on your behalf.

Upgrading later is `brew upgrade --cask sakina`.

### First run

Sakina asks for two permissions, and explains itself before each:

- **Notifications** — without these there are no reminders, which is most of
  the app
- **Location** — only to work out prayer times. You can search for your city
  instead and never grant it. Either way your coordinates stay on the Mac.

It also adds itself to your login items, so it is there after a restart. That
is a switch in Settings if you would rather it were not.

[latest]: https://github.com/asadalihaider/pray-with-sakina/releases/latest

## Known limitations

These are real, and most of them are not fixable from inside an app.

| | |
|---|---|
| Reminders are suppressed during **Focus / Do Not Disturb** | macOS only allows an app past Focus with the Time Sensitive entitlement, which needs a paid developer account and the user opting in. Not something a setting can force. |
| Builds are **not notarised** | See [Install](#install). Signing is free and the app is signed; notarisation is the paid part. |
| Reminders only **wait on screen** if you pick **Alerts** | macOS defaults every app to *Banners*, which dismiss themselves after a few seconds. No API can change that default — the style belongs to you. Onboarding sends a test reminder and links straight to the setting. |
| The panel does not appear over a **fullscreen app** | Apple's own SwiftUI `MenuBarExtra` has the same problem, and the documented `presentationOptions` workaround is ignored. |
| An auto-hidden **menu bar** still hides behind the panel | macOS reveals it for the pointer or a real `NSMenu`, and a web view can be neither. |
| The tray icon has **no right-click menu**; Quit is on a **double click** | `tray-icon` 0.24 attaches a menu to the status item permanently, and macOS 27 pops it on left click, swallowing every attempt to open the panel. The menu is built and shown by hand instead. |
| Pausing reminders while the **mic or camera** is in use is a setting that exists but does nothing yet | |

**macOS only.** The notification path is an empty stub on other platforms, so a
Windows or Linux build would run without the feature it exists for.

## Building it

Needs [Rust](https://www.rust-lang.org/tools/install) and Node 20+.

```bash
npm install
npm run tauri dev
```

Reminders will not behave under `tauri dev`: an unbundled binary is not
registered with Launch Services, so macOS misattributes the notification and a
click cannot route back to the app. Test them against a real bundle instead —
`./reinstall.sh` quits, builds, replaces the installed copy and relaunches, and
`./reinstall.sh --fresh` also wipes your records so onboarding runs from the top.

Tests — 76 of them, covering the prayer time maths, window rules, the nudge
schedule, the storage layer, the review floor, and the settings migrations:

```bash
cd src-tauri && cargo test
```

### The signature is not optional

`UNUserNotificationCenter` refuses an ad-hoc signed app outright: asking for
permission returns denied without ever showing a prompt, and delivery fails
with `UNErrorDomain` 1, "notifications not allowed".

Any ordinary signature satisfies it — it does **not** have to come from a paid
Apple Developer account. A self-signed certificate works, and that is what
both local builds and releases use:

```bash
# once, then it lives in your login keychain
openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout key.pem -out cert.pem \
  -subj "/CN=Sakina Signing/O=Sakina" \
  -addext "basicConstraints=critical,CA:false" \
  -addext "keyUsage=critical,digitalSignature" \
  -addext "extendedKeyUsage=critical,codeSigning"
openssl pkcs12 -export -inkey key.pem -in cert.pem -out id.p12 \
  -passout pass:CHANGEME -name "Sakina Signing" \
  -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg sha1
security import id.p12 -k ~/Library/Keychains/login.keychain-db -P CHANGEME -A
```

The legacy PKCS#12 algorithms are deliberate: macOS cannot read what OpenSSL 3
produces by default.

Signing also turns on the hardened runtime, which is why
[`Entitlements.plist`](src-tauri/Entitlements.plist) exists. Without
`com.apple.security.personal-information.location` in it, `locationd` refuses
to raise the permission prompt at all — no error, no dialog, nothing.

macOS keys both permissions to the bundle identifier **and** the signing
certificate, so a build signed with a different certificate is a different app
as far as the system is concerned: permissions reset and the prompts return.
Releases are therefore always signed with the same certificate.

## How it is put together

Tauri 2, with a React + TypeScript front end and the logic in Rust.

```
src-tauri/src/
  engine/      prayer times, windows, nudge schedule, prayer state — pure
               functions with no clock of their own, so tests can fix time
  scheduler.rs decides when a reminder is due
  notifier.rs  UNUserNotificationCenter: permission, delivery, the Prayed button
  store.rs     SQLite: prayer logs, Qaza entries, Surah Mulk, settings
  settings.rs  settings, and the migrations that got them here
  today.rs     builds what the Today tab shows
  stats.rs     the month grid, streaks and the Qaza ledger
  location.rs  CoreLocation, and naming a place without disclosing it
  icon.rs      the menu bar star, rasterised at runtime
src/           the panel: Today, Stats, Qaza, Settings, onboarding
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

## Licence

MIT. See [LICENSE](LICENSE).
