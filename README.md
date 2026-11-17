# TinyHand for Windows

A tiny hand on a stick that points at things on your screen. Made for presenting, teaching
and screen sharing.

This is an unofficial Windows port, written in Rust, of the macOS app
[TinyHand](https://github.com/nilbuild/tinyhand) by Kamran Ahmed. The idea, the design, the
hands and the sounds are all from the original; see [Credits](#credits).

## Install

Needs Windows 10 or 11, 64-bit.

1. Download `tinyhand.exe` from the [latest release](../../releases/latest).
2. Put it anywhere you like, for example a `TinyHand` folder in your Documents.
3. Double-click it. A hand icon appears in the notification area (the tray) next to the clock.

That is the whole installation. It is a single file with every hand, sound and icon inside it,
and it needs no installer, no runtime and no administrator rights.

- The executable is not code-signed, so the first time you run it Windows SmartScreen may say
  "Windows protected your PC". Choose **More info**, then **Run anyway**.
- To start TinyHand with Windows, right-click the tray icon, open **Settings** and turn on
  "Open TinyHand when you log in". If you later move the exe, turn that off and on again.
- To update, quit TinyHand from the tray menu and replace the file.
- To remove it, turn off "Open TinyHand when you log in", quit, and delete the file. The
  leftovers are listed under [Where things are kept](#where-things-are-kept).

## Use

Run `tinyhand.exe`. It sits in the notification area (the tray) next to the clock.

| | |
|---|---|
| Click the tray icon, or **Ctrl+Alt+H** | Turn the pointer on or off |
| Right-click the tray icon | Menu and Settings |
| Click | Tap (nothing underneath gets clicked) |
| Hold and drag | Draw on the screen |
| Hold **Ctrl** while drawing | Add an arrowhead to the end of the line |
| **Ctrl+Alt+X** | Clear the drawing |
| Hold **Alt** and click | Click for real, on the app underneath |
| Double-tap **Shift** | Let every click through until you double-tap again (the tray icon turns orange) |
| **Esc** | Clear everything and turn the pointer off |

While the pointer is on, a small toolbar near the bottom of the screen lets you pick a hand or
the laser dot, change the ink colour, mute the tap sound, or exit. Move the mouse to the bottom
of the screen to bring it back.

Over the taskbar, open menus and TinyHand's own windows the normal cursor returns and clicks
work as usual.

Settings (shortcuts, the 36 hands, size, shadow, ink, sounds, your own hands) are in the tray
menu. Starting `tinyhand.exe` a second time opens Settings.

## Automation

Launchers and scripts can control TinyHand with `tinyhand://` links, or by running the
executable with a command:

```bat
start tinyhand://toggle
tinyhand.exe style/robot
```

| Link | Action |
|---|---|
| `tinyhand://on`, `tinyhand://off`, `tinyhand://toggle` | Turn the pointer on, off, or flip it |
| `tinyhand://clear` | Clear the drawing |
| `tinyhand://style/<id>` | Switch to a hand (or `laser`) and turn the pointer on |
| `tinyhand://sound/on`, `/off`, `/toggle` | Tap sound for the current pointer |
| `tinyhand://settings` | Open Settings |
| `tinyhand://quit` | Quit TinyHand |

The link handler is registered for the current user the first time TinyHand runs. If TinyHand
isn't running, a link starts it and then runs the command.

While TinyHand runs it keeps its state in `%APPDATA%\TinyHand\state.json`: whether the pointer
is on, the selected style, whether sound is on, and every style with its `id`, name and type
(`hand`, `custom` or `laser`). The file is removed when TinyHand quits. It also records the
process id, so a file left behind by a crash can be detected.

## Where things are kept

| | |
|---|---|
| `%APPDATA%\TinyHand\settings.json` | Your settings |
| `%APPDATA%\TinyHand\Hands\` | Hands you made yourself |
| `HKCU\Software\Classes\tinyhand` | The `tinyhand://` link handler |
| `HKCU\...\CurrentVersion\Run` → `TinyHand` | Only if "Open TinyHand when you log in" is on |

## Good to know

- Windows does not let a normal program see input meant for a program running as
  administrator. While such a window is in front, clicks go to it rather than becoming taps.
  Run TinyHand as administrator too if you need to present such apps.
- The pointer turns itself off when the screen locks or the PC goes to sleep.
- The cursor is hidden through the Windows Magnification API. Windows brings it back by itself
  if TinyHand ever exits unexpectedly.

## Differences from the macOS app

- No Accessibility permission step; Windows doesn't need one.
- No self-updater.
- Modifier keys: Alt replaces Option as the "click for real" key (Ctrl and Win are also
  offered), and Ctrl replaces Command for the arrowhead.
- The toolbar is drawn on the overlay itself rather than being a separate window.

## Build

Needs [Rust](https://rustup.rs) (stable, MSVC toolchain) and the Visual Studio C++ build tools.

```bat
cargo build --release
```

The result is `target\release\tinyhand.exe`. The C runtime is linked in (see
`.cargo\config.toml`), so it runs on machines without the Visual C++ Redistributable.

For an executable you are going to give to other people, use the release script instead. It
does the same build, but also keeps folder names from your machine out of the file and prints
its SHA-256:

```bat
powershell -ExecutionPolicy Bypass -File build-release.ps1
```

### Checks

```bat
cargo test
target\release\tinyhand.exe --render-test out.png lavender
target\release\tinyhand.exe --settings-test out.png general
target\release\tinyhand.exe --e2e-test report.txt
```

- `cargo test` runs the unit tests.
- `--render-test` draws a sample scene to a PNG without touching the screen.
- `--settings-test` draws one page of the settings window to a PNG.
- `--e2e-test` turns the real pointer on and drives it with synthetic input for about ten
  seconds (it takes over the mouse while it runs), then writes a pass/fail report.
- `--make-hand in.png out.png` runs the "make your own hand" check and composition on a file.

### Source layout

| | |
|---|---|
| `src/main.rs` | Entry point, command-line tools, single-instance hand-off |
| `src/app.rs` | Tray icon, shortcuts, activation, the per-frame loop |
| `src/hook.rs` | Low-level mouse and keyboard hooks (own thread) |
| `src/render.rs` | Direct2D / DirectComposition drawing |
| `src/arm.rs`, `src/ink.rs`, `src/toolbar.rs` | The hand's pose, drawn lines, the toolbar |
| `src/settings*` | The settings window |
| `src/handmaker.rs`, `src/custom.rs`, `src/handasset.rs` | Custom hands and image measuring |
| `src/prefs.rs`, `src/styles.rs`, `src/sound.rs`, `src/cursor.rs`, `src/registry.rs`, `src/automation.rs` | Settings, styles, sound, cursor hiding, registry, `tinyhand://` |
| `assets/` | Hands (WebP), thumbnails, sounds, icons — embedded at build time |
| `tools/convert-assets/` | Regenerates `assets/` from the original macOS artwork |

## Credits

TinyHand was created by Kamran Ahmed. The original macOS
app lives at [nilbuild/tinyhand](https://github.com/nilbuild/tinyhand). If you are on a Mac,
use that one.

Everything that makes TinyHand what it is comes from the original: the idea, the behaviour,
the 36 hand pictures, the tap sounds and the icon. This repository only re-implements the
program for Windows. The artwork under `assets/` is the original artwork, recompressed.

This port is not affiliated with or endorsed by the original author. Please report problems
with the Windows version here, not to the original project.

## License

MIT, the same as the original. See [LICENSE](LICENSE), which keeps the original copyright
notice: © 2026 Kamran Ahmed.
