# Open ADI-2 Control

Remote control for RME ADI-2 series converters on Linux: ADI-2 DAC, ADI-2 Pro, ADI-2/4 Pro SE
and their FS/EX versions. It talks to the unit over its USB-MIDI port, the same way RME's ADI-2
Remote does on Windows, macOS and iPadOS. It can also control a unit that is connected to an
[ADI-2-Pi](https://github.com/shells-dw/adi2-pi) on the network.

Not affiliated with RME. "RME" and "ADI-2" are trademarks of RME Intelligent Audio Solutions.

![Line Out page with a parametric EQ preset loaded](docs/screenshot.png)

## Features

- **Outputs** (Line Out, Phones 1/2, Phones 3/4; ADI-2 DAC: Line Out, Phones, IEM): source,
  volume, Lock, Dim, Mute, balance, reference level, parametric EQ with curve, Bass/Treble,
  Dual EQ, width, crossfeed, DA filter, mono, polarity, M/S, loudness, DC protection,
  de-emphasis, loopback, rear TRS source (ADI-2/4 Pro SE).
- **Line input** (Pro models): conversion, reference level, trim, AD filter, DC filter,
  polarity, M/S, RIAA, EQ.
- **EQ**: 5 bands plus Bass/Treble as knobs and as points on the curve (drag for frequency and
  gain, mouse wheel on a point for Q), L/R when Dual EQ is on, presets selectable per output,
  storing the current EQ into one of the unit's 20 presets.
- **EQ presets**: editor for the unit's 20 presets without touching the active EQ; Listen
  (send to an output) with A/B (switch the output between its previous EQ and the preset),
  Copy/Paste, Clear, Re-load, Store with name. Import and export of ADI-2 Remote preset files
  (`.adieqpr`).
- **Headphone EQ**: an EQ for the five bands (optionally Bass/Treble too) from AutoEq's
  headphone measurements and a target curve, or from your own measurement file; apply it to an
  output, store it as a preset, open it in the preset editor or save it as `.adieqpr`.
- **Device**: basic mode, clock, SPDIF/SRC/optical, phones options, DSD, display, key
  remapping, auto standby, sample rate (read only), 9 setups (load, save with name, rename),
  factory settings.
- **Volume options** like ADI-2 Remote's: limit per output, Recall volume per output, Link
  (volumes move together), limited rise speed (5/10/20 dB/s). Several sets of limits
  (profiles), switchable in the tray menu.
- **Volume keys** control the ADI-2 (see below), **tray icon** with volume, Mute, Dim, EQ and
  setups, **volume popup** for key presses and for turns of the knob on the unit, **start at
  login**.
- **Several units**: saved connections (USB, ADI-2-Pi bridges, Demo), one active at a time,
  switched in the top bar or the tray menu.
- **Unit status** (Settings → Unit): device mode, balanced phones, DSD, AES/SPDIF input signal
  (Pro models), the raw status message.
- **Command line and D-Bus** control, **keyboard shortcuts**, dark and light theme, compact
  layout.
- The page follows the output whose volume is turned on the unit (can be switched off).
- Only shows what the connected unit reports (an ADI-2 Pro EX sends no Phones 1/2 Mute, for
  example).

Changes made on the unit (knob, keys, IR remote) appear in the app. The unit doesn't report
changes sent over MIDI back, so the app reads the settings back 1.5 s after a change and
re-reads everything every 30 s, after the computer wakes from sleep and after the screen is
unlocked.

## Requirements

- Firmware with the remote protocol, as for RME's ADI-2 Remote 2.2: ADI-2 DAC USB 81 / DSP 61,
  ADI-2 Pro USB 267 / DSP 127, ADI-2/4 Pro SE USB 72 (hardware revision 6) or 209 (revision 7) /
  DSP 39, ADI-2/4 Pro EX USB 315 / DSP 105, or newer.
- On the unit: **MIDI Control = ON** (menu Device Mode / DSD; ADI-2 DAC: Remap Keys/Diag).
- Linux with the ALSA sequencer (kernel module `snd-seq`), WebKitGTK 4.1. For the tray icon:
  libayatana-appindicator and a panel that shows tray icons (GNOME needs the AppIndicator
  extension; Ubuntu has it).

## Install

Download from [Releases](https://github.com/shells-dw/open-adi2-control/releases): `amd64` for
x86-64 PCs, `arm64` for 64-bit ARM (e.g. Raspberry Pi 4/5 with a desktop).

**Debian, Ubuntu, Mint, Pop!_OS** (Ubuntu 22.04 / Debian 12 or newer):

```sh
sudo apt install ./open-adi2-control_0.2.0_amd64.deb
```

**Fedora, openSUSE** (`.rpm`, `x86_64` / `aarch64`):

```sh
sudo dnf install ./open-adi2-control-0.2.0-1.x86_64.rpm
```

**Arch Linux**: `packaging/aur/PKGBUILD` builds from the release tag (see
`packaging/README.md`).

**Other distributions**: the AppImage (glibc 2.35 or newer).

```sh
chmod +x open-adi2-control_0.2.0_amd64.AppImage
./open-adi2-control_0.2.0_amd64.AppImage
```

A Flatpak manifest is in `packaging/flatpak/`; it is not on Flathub.

Each volume key press starts the AppImage briefly, so with the volume keys the `.deb` (or a
build from source) reacts faster.

It appears as *Open ADI-2 Control* in the application menu. To update, install the new version over
the old one; settings are kept.

## Connection

Settings → Connection. *Add* creates another saved connection; with more than one, the top bar
and the tray menu switch between them. Each has its own type, address and key.

- **USB on this computer** (default). The app uses the ALSA sequencer, so it works next to
  PipeWire, JACK and other MIDI programs; nothing has to be switched off. It finds the unit by
  its name and notices when it is plugged in or out.
- **ADI-2-Pi on the network** ([ADI-2-Pi](https://github.com/shells-dw/adi2-pi): an ADI-2
  connected to a Raspberry Pi): *Find* lists the ADI-2-Pi bridges on the network (mDNS
  `_adi2._tcp`), or type the address (`adi2-pi.local`, `192.168.1.20`, `host:8080`). The app
  uses the bridge's API (version 1); nothing on the Pi has to change. The bridge's own volume
  limits apply in addition to the app's.
  If an access key is set on the ADI-2-Pi (its System tab), enter it under *Access key*. A
  missing or wrong key stops the connection with a message instead of retrying; after 10 wrong
  keys within a minute (from any client) the bridge refuses connections for a minute, and the
  app waits that long. The key is kept in the settings file, readable only by your user.
  A bridge set to *key needed for changes only* (ADI-2-Pi 1.0.0) shows its state without a key:
  the app then marks the connection **View only** in the top bar and asks for the key at the
  first change (or when the badge is clicked).
- **Demo**: a simulated ADI-2/4 Pro SE, ADI-2 Pro or ADI-2 DAC.

## Volume keys

Settings → Volume keys → *Volume keys control the ADI-2*.

The keys then change the volume of one output of the ADI-2, by the step set under Settings →
Volume (default 1 dB):

- **Output**: the output turned last (on the unit or in the app), or a fixed one. Also in the
  tray menu.
- **When**: *While the ADI-2 is the sound output* (default): only while the computer's default
  output is the ADI-2; with any other output (Bluetooth headphones, HDMI …) the keys change the
  system volume as usual. *Always*: always the ADI-2. With ADI-2-Pi, the keys always control the
  ADI-2.

Desktops don't let an app take keys away from them, so the keys are bound to commands:

```
open-adi2-control volume-up
open-adi2-control volume-down
open-adi2-control mute
```

The command passes the key to the running app and returns at once. If the app isn't running,
it changes the system volume.

**GNOME** (also Ubuntu, Pop!_OS, Zorin): the app sets the bindings up itself when the option is
switched on, and restores GNOME's own when it is switched off. Switch it off before removing the
app.

**Other desktops**: bind the keys to the commands yourself. Settings → Volume keys shows the
commands with the full path (for the AppImage, the path of the AppImage file).

- **KDE Plasma**: System Settings → Keyboard → Shortcuts → *Audio Volume*: remove the shortcuts
  of *Increase Volume*, *Decrease Volume* and *Mute*. Then *Add New → Command or Script* three
  times with the commands, each bound to its key.
- **Xfce**: in the PulseAudio panel plugin's properties switch off *Enable keyboard shortcuts
  for volume control*, then Settings → Keyboard → Application Shortcuts → *Add*.
- **Cinnamon**: Keyboard → Shortcuts → *Sound and Media*: remove the volume keys; then *Custom
  Shortcuts → Add custom shortcut*.
- **sway / i3**:
  ```
  bindsym XF86AudioRaiseVolume exec open-adi2-control volume-up
  bindsym XF86AudioLowerVolume exec open-adi2-control volume-down
  bindsym XF86AudioMute exec open-adi2-control mute
  ```
- **Hyprland**:
  ```
  bind = , XF86AudioRaiseVolume, exec, open-adi2-control volume-up
  bind = , XF86AudioLowerVolume, exec, open-adi2-control volume-down
  bind = , XF86AudioMute, exec, open-adi2-control mute
  ```

The volume limit and rise speed apply to the keys too.

## Keyboard shortcuts

In the window (not while typing in a field): **↑/↓** volume by the step, **Page ↑/↓** volume by
5 dB, **M** Mute, **D** Dim, **E** EQ on/off, **1–9** load a setup (asks first),
**Ctrl+Page ↑/↓** previous/next tab, **?** the list. They act on the output of the open page,
otherwise on the output the volume keys control.

## Command line and D-Bus

While the app runs, the same binary controls it from scripts:

```sh
open-adi2-control get                     # everything, as JSON
open-adi2-control get ph12                # one output
open-adi2-control get line vol            # one value: -20.0
open-adi2-control set ph12 vol -30        # absolute
open-adi2-control set ph12 vol +1.5       # relative: +N, -=N
open-adi2-control set line mute toggle    # on, off, toggle
open-adi2-control set line src next       # a choice by name or number, next, prev
open-adi2-control set line b1g 2.5 R      # right EQ side with Dual EQ
open-adi2-control setup 2                 # load setup 2 (no confirmation)
open-adi2-control preset ph12 5           # EQ preset 1–20, manual, temp
open-adi2-control refresh                 # read everything again
```

Parameter ids are those in `src-tauri/schema.json` (`vol`, `mute`, `dim`, `src`, `eqen`,
`eqpreset`, `b1g`/`b1f`/`b1q` … for EQ bands, `bass`, `treble`); channels are `line`, `ph12`,
`ph34`, `in`, `device`. Values are in display units (dB, Hz). Output goes to stdout; errors to
stderr with exit code 1, exit code 2 when the app isn't running. Volume limits apply.

The same commands over D-Bus (session bus), name and object
`io.github.shells_dw.OpenAdi2Control` / `/io/github/shells_dw/OpenAdi2Control`, interface
`io.github.shells_dw.OpenAdi2Control1`, methods `Run(as) → s` and `Get(s channel, s id) → s`:

```sh
busctl --user call io.github.shells_dw.OpenAdi2Control /io/github/shells_dw/OpenAdi2Control \
  io.github.shells_dw.OpenAdi2Control1 Run as 4 set ph12 vol -30
```

## EQ preset files

EQ Presets → *Import…* reads ADI-2 Remote's `.adieqpr` files (also those written by ADI-2-Pi's
headphone EQ) and lists their presets with a target slot each; *Edit* opens one in the editor,
*Store selected* writes the chosen ones to the unit. *Export…* writes all stored presets of the
unit into one `.adieqpr` file. ADI-2 Remote's setup files (`.adistp`) are not supported.

A preset stored without a name gets the name "Preset N" (N = its slot).

## Headphone EQ

The Headphone EQ tab fits the ADI-2's EQ to a headphone: pick a headphone (AutoEq's
oratory1990 measurements, 736 models, shipped with the app) or load a measurement file (CSV or
text: frequency and level), and a target curve (Harman 2018 for over-ear and 2019 for in-ear by
default, or a file). *Options*: bass shelf and tilt added to the target, a frequency above which
nothing is corrected, number of bands, maximum boost and cut, band 1/5 types, Bass/Treble as
extra bands, compensation of the Slow DA filter.

The chart shows the deviation from the target before and after the EQ. The result can be
applied to an output (as Manual EQ, after a confirmation), stored as a preset, opened in the
preset editor (for Listen and A/B) or saved as `.adieqpr`. The fitting code is ADI-2-Pi's
headphone EQ (`ui/hp/core.js`) and runs locally.

## Volume popup

Shows the output and its volume after a key press, and when the volume is turned on the unit
(Settings → Volume popup). Style *Automatic* picks:

- KDE Plasma: Plasma's own volume display,
- X11: a small window at the bottom of the screen,
- GNOME and other Wayland desktops: a desktop notification that replaces the previous one
  (Wayland doesn't let apps place windows; GNOME doesn't let apps use its own volume display).

## Tray and start at login

The tray icon's menu shows the volume of the output the keys control, has Volume up/down,
Mute, Dim and EQ for that output, which output the keys control, the volume limit profiles and
saved connections (when there is more than one), and the 9 setups (loading one opens the window
to confirm). With
*Closing the window keeps the app in the tray*, closing the window doesn't quit the app (Quit
in the tray menu does). *Start at login* adds `~/.config/autostart/open-adi2-control.desktop`;
*Start without the window* starts it in the tray only.

## Updates

Settings → App → *Check GitHub for new versions* is **off** unless you switch it on. When on,
the app asks GitHub about 20 seconds after it starts and then once a day for this project's
latest published release. *Check now* asks once, also while the switch is off.

When a newer version exists, a bar at the top of the window, the tray menu and a desktop
notification say so. *What's new* shows the release notes, a download link for the package that
matches this installation (`.deb`, `.rpm` or AppImage, `amd64` or `arm64`) and how to install
it (AUR and Flatpak installs: a hint to update through those).
*Skip this version* hides that version until a newer one comes out.

The app never downloads or installs anything by itself. The check sends one request to
`api.github.com` (your IP address and `open-adi2-control/<version>` as user agent), nothing
else. A proxy set in `HTTPS_PROXY` or `ALL_PROXY` is used.

## Files

- Settings: `~/.config/open-adi2-control/config.json` (0.1.x settings are converted on the
  first start of 0.2.0)
- Start at login: `~/.config/autostart/open-adi2-control.desktop`
- Command socket while running: `$XDG_RUNTIME_DIR/open-adi2-control.sock`
- Headphone data: `eq-data/` next to the app's resources (`/usr/lib/open-adi2-control/eq-data`
  for the packages)

The app writes nothing to the unit when it starts; it only reads. Values change on the unit
when you change them.

## Troubleshooting

- **No ADI-2 found**: `aconnect -l` should list a client named like `ADI-2 Pro (12345678)`. If
  it doesn't, check the USB connection and `lsusb`. If it does, set MIDI Control to ON.
- **Found but not answering**: MIDI Control is OFF, or the firmware is too old for the remote
  protocol.
- **Can't open the ALSA sequencer**: `sudo modprobe snd-seq`; your user needs access to
  `/dev/snd/seq` (desktop logins have it; otherwise the `audio` group).
- **Tray icon missing on GNOME**: install/enable the *AppIndicator and KStatusNotifierItem
  Support* extension. Without a tray, a closed window comes back by starting the app again from
  the application menu.
- **A value jumps back after a change**: the unit refused it (a setting that doesn't apply in the
  current mode, for example). The app reads the unit back 1.5 s after a change and shows what
  it has.
- **AppImage renamed or replaced**: the autostart entry and the GNOME key bindings are updated
  to the new file when the app starts.

## Building

Needs Rust (1.77+), Node.js (for the Tauri CLI) and on Debian/Ubuntu:

```sh
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libasound2-dev libssl-dev build-essential file
npm install
npx tauri build            # AppImage, .deb and .rpm in src-tauri/target/release/bundle/
```

Releases: pushing a tag `vX.Y.Z` runs `.github/workflows/release.yml`. It builds on Ubuntu
22.04 for amd64 and arm64 and attaches the packages and checksums to a draft release with the
text of `RELEASE-NOTES-X.Y.Z.md`. AUR and Flatpak: `packaging/README.md`.

`cargo test` in `src-tauri` runs the protocol tests, the `.adieqpr` reader/writer, settings
conversion, command-line parsing and an end-to-end test of the USB engine against the simulated
unit. `dbus-run-session -- cargo test -- --ignored gnome` tests the GNOME key bindings (needs
gsettings and the GNOME media-keys schema). The tests in `pi::live_tests` / `pi::view_tests`
(`--ignored`) need a bridge on 127.0.0.1:8098 (key `s3cret`) and 127.0.0.1:8097 (*key needed
for changes only*).

### Layout

| Path | |
|---|---|
| `src-tauri/src/protocol.rs` | SysEx codec |
| `src-tauri/src/device.rs` | the unit over its MIDI port: identify, read out, set, read back, presets, setups |
| `src-tauri/src/seq.rs` | ALSA sequencer port |
| `src-tauri/src/mock.rs` | simulated unit (Demo, tests) |
| `src-tauri/src/pi.rs` | ADI-2-Pi client (API version 1) and discovery |
| `src-tauri/src/hub.rs` | connection, model, volume limit/link/rise, popup and tray updates |
| `src-tauri/src/keys.rs` | command socket, system volume, GNOME key bindings, host programs from an AppImage |
| `src-tauri/src/dbus.rs` | D-Bus interface |
| `src-tauri/src/wake.rs` | re-read after resume and screen unlock |
| `src-tauri/src/eqfile.rs`, `hpeq.rs` | `.adieqpr` files, headphone data |
| `src-tauri/src/osd.rs`, `tray.rs` | popup, tray |
| `src-tauri/src/update.rs` | update check against GitHub's releases API |
| `src-tauri/src/config.rs`, `model.rs`, `schema.rs` | settings file, shared types, parameter lookups and command-line values |
| `src-tauri/schema.json` | parameters, ranges and value names per model |
| `ui/app.js`, `ui/eq.js` | the window, EQ curve math |
| `ui/hp.js`, `ui/hp/` | Headphone EQ tab, fitting code and worker (from ADI-2-Pi) |
| `eq-data/` | AutoEq measurements and targets (from ADI-2-Pi) |
| `packaging/` | AUR, Flatpak, desktop file, AppStream metainfo |

## Not included yet

- ADI-2 Remote's setup files (`.adistp`).
- ADI-2 Remote's full status window (per-input sync, sample rate and bit depth, output state):
  the status message's input and output fields are only partly documented.
- Switching the page when phones are plugged in (Pro models): the status message's active
  output field is not confirmed for them; the page follows the knob instead.

## License

GPL-3.0-or-later. See `LICENSE` and `NOTICE.md`.
