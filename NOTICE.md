# Notices

- **RME ADI-2 MIDI protocol**: the SysEx protocol is RME's ("MIDI-Protocol for RME ADI-2 DAC,
  ADI-2 Pro and ADI-2/4 Pro SE", published with ADI-2 Remote). This app implements it
  independently; it contains no RME code.
- **Parameter list** (`src-tauri/schema.json`): taken from ADI-2-Pi
  (https://github.com/shells-dw/adi2-pi (currently unreleased, tbd), GPL-3.0-or-later), which compiled it from RME's MIDI
  table, the ADI-2 manuals and tests on an ADI-2 Pro EX.
- **EQ curve**: RBJ "Audio EQ Cookbook" (Robert Bristow-Johnson) filter formulas.
- **Tauri** (MIT/Apache-2.0) and the Rust crates listed in `src-tauri/Cargo.lock` under their
  own licenses.
- **Trademarks**: RME, ADI-2 and ADI-2 Remote are trademarks of RME Intelligent Audio
  Solutions. This app is not affiliated with or endorsed by RME.
