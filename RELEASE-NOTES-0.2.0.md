## New

- **Headphone EQ** tab: an EQ for the ADI-2's five bands (optionally Bass/Treble) from AutoEq's
  headphone measurements (736 models, shipped with the app) or your own measurement file, and a
  target curve. Apply it to an output, store it as a preset, open it in the preset editor or
  save it as `.adieqpr`. Same fitting as ADI-2-Pi's headphone EQ.
- **EQ preset files**: import and export of ADI-2 Remote's `.adieqpr` files (EQ Presets →
  Import…/Export…).
- **A/B** in the preset editor: after *Listen*, switch the output between its previous EQ (A)
  and the preset (B).
- **Several units**: saved connections, switched in the top bar or the tray menu.
- **Volume limit profiles**, switchable in the tray menu.
- **Command line and D-Bus control**: `open-adi2-control get|set|setup|preset|refresh …`, and the
  same over D-Bus (`io.github.shells_dw.OpenAdi2Control`). See the README.
- **Keyboard shortcuts** in the window (↑/↓, Page ↑/↓, M, D, E, 1–9, Ctrl+Page ↑/↓, ?).
- **Light theme** and **compact layout** (Settings → App); dark stays the default.
- The page **follows the output** whose volume is turned on the unit (Settings → App, on by
  default).
- **Unit status** in Settings → Unit: device mode, balanced phones, DSD, AES/SPDIF input signal
  (Pro models), raw status words.
- Everything is **read again after resume** from sleep and after the screen is unlocked.
- **ADI-2-Pi 1.0.0 "key needed for changes only"**: without a key the app shows the unit
  read-only with a *View only* badge and asks for the key at the first change. A link to
  ADI-2-Pi next to the connection setting.
- `.rpm` packages for Fedora/openSUSE; an AUR PKGBUILD and a Flatpak manifest in `packaging/`.

## Changed

- Loading a setup asks for confirmation (window, tray menu, shortcut).
- A preset stored without a name is named "Preset N" (its slot).
- Settings from 0.1.x are converted on the first start (connection → saved connections, volume
  limits → the profile "Default").

## Fixed

- ADI-2-Pi: commands were sent up to 50 ms late; the client now wakes for them at once.
