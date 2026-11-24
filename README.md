# Fuzzie

A small, fast desktop notch and Bongo-style taskbar companion for Windows, written in Rust with Direct2D.

- **Notch:** clock, now playing with media controls, synced lyrics that slide in line by line, a file shelf and clipboard history.
- **HUDs:** volume, Caps Lock, audio device, and Windows notifications (read from the local notification database).
- **Fuzzie companions:** 5 fluffy animal species (**Fluffy Fox**, **Mochi Cat**, **Puff Bunny**, **Chibi Shiba**, **Red Panda**) drawn entirely in code, with 59 skins and dual **Head + Gear** accessories. Sit your Fuzzie on the taskbar or snap it to any screen edge — it taps on every key press and click, reacts happily as you type faster, smoothly curls up to nap, and rolls onto its back to bat a yarn ball.
- **Friends:** add friends with a code like `FUZ-A4G7B9`, see who is online and what they are playing, send a **Poke** or **Kiss** (their notch shows your Fuzzie), listen along, or try it immediately with the built-in **Fuzzie Bot**. No email or password: just a name and a code.
- **Light:** a few MB of memory and near-zero CPU when idle.

## Download

See it on the [Fuzzie page](https://ismailsenn.github.io/fuzzie-notch/), or grab `Fuzzie.exe` from the [latest release](https://github.com/ismailsenn/fuzzie-notch/releases/latest). It keeps itself up to date from this repository's releases; turn that off in Settings → Updates → "Update automatically".

## Build

```powershell
cargo build --release
```

The executable is `target\release\fuzzie.exe`. Windows 10/11, x64.

The friends relay lives in `server/` (a Cloudflare Worker with one Durable Object):

```powershell
cd server
npm install
npx wrangler dev      # local relay; run Fuzzie with FUZZIE_RELAY=http://127.0.0.1:8787
npx wrangler deploy
```

## Notes

Skins and accessories come in packs (Classic, Shinobi Pack); every look is drawn in code for Fuzzie. Built by Daicho.
