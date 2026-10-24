Remote control for RME ADI-2 series converters on Linux: ADI-2 DAC, ADI-2 Pro and ADI-2/4 Pro SE, including the FS and EX versions. Controls the unit over its USB-MIDI port through the ALSA sequencer, next to PipeWire or JACK. Not affiliated with RME.

**Pages**
- Line In and all outputs: source, volume, Lock, Dim, Mute, balance, reference level, parametric EQ with curve, Bass/Treble, Dual EQ, width, crossfeed, filters, mono, polarity, M/S, loudness, DC protection, de-emphasis, loopback.
- Device: modes, clock, digital I/O, phones, DSD, display, key remapping, auto standby, 9 setups (load, save with name, rename), factory settings.
- EQ Presets: editor for the unit's 20 presets (Listen, Copy/Paste, Clear, Re-load, Store with name).
- Volume limit, Recall volume, Link and rise-speed limit per output.
- Demo mode with a simulated ADI-2/4 Pro SE, ADI-2 Pro or ADI-2 DAC.

**Desktop**
- Volume keys control the ADI-2 while it is the sound output; set up automatically on GNOME, bindable commands for other desktops.
- Tray menu: volume, Mute, Dim, EQ, output for the keys, setups.
- Volume popup for key presses and for turns of the knob on the unit.
- Start at login.

**Packages**
- `.deb` for Ubuntu 22.04, Debian 12 and newer.
- AppImage for other distributions (glibc 2.35 or newer).
- Both for `amd64` and `arm64`. `SHA256SUMS-*.txt` lists the checksums.

**Requirements on the unit**: firmware with remote support (as for RME's ADI-2 Remote) and MIDI Control = ON (menu Device Mode / DSD; ADI-2 DAC: Remap Keys/Diag).

**Not included**: setup and EQ preset files, the status window, automatic switching to the page of the active output.
