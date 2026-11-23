# Fuzzie

A small, fast desktop notch for Windows, written in Rust with Direct2D.

- **Notch:** clock, now playing with media controls, synced lyrics that slide in line by line, a file shelf and clipboard history.
- **HUDs:** volume, Caps Lock, audio device, and Windows notifications (read from the local notification database).
- **Fuzzie:** a chubby fox mascot drawn entirely in code, with dozens of skins and accessories. It can also sit on your taskbar and tap a paw on every key press and click, Bongo Cat style.
- **Light:** a few MB of memory and near-zero CPU when idle.

## Download

Grab `Fuzzie.exe` from the [latest release](https://github.com/ismailsenn/Fuzzie/releases/latest). It keeps itself up to date from this repository's releases; turn that off in Settings → General → "Update automatically".

## Build

```powershell
cargo build --release
```

The executable is `target\release\fuzzie.exe`. Windows 10/11, x64.

## Notes

The "shinobi" skins and accessories are fan-made tributes; character names belong to their owners.
