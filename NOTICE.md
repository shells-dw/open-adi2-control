# Notices

- **RME ADI-2 MIDI protocol**: the SysEx protocol is RME's ("MIDI-Protocol for RME ADI-2 DAC,
  ADI-2 Pro and ADI-2/4 Pro SE", published with ADI-2 Remote). This app implements it
  independently; it contains no RME code.
- **Parameter list** (`src-tauri/schema.json`): taken from ADI-2-Pi
  (https://github.com/shells-dw/adi2-pi, GPL-3.0-or-later), which compiled it from RME's MIDI
  table, the ADI-2 manuals and tests on an ADI-2 Pro EX.
- **EQ curve**: RBJ "Audio EQ Cookbook" (Robert Bristow-Johnson) filter formulas.
- **Headphone EQ fitting** (`ui/hp/core.js`, `ui/hp/worker.js`): from ADI-2-Pi's headphone EQ
  (https://github.com/shells-dw/adi2-pi, GPL-3.0-or-later).
- **Headphone measurements and target curves** (`eq-data/`): from AutoEq by Jaakko Pasanen
  (https://github.com/jaakkopasanen/AutoEq, MIT), measurements by oratory1990; the snapshot and
  the index file as shipped with ADI-2-Pi (AutoEq commit noted in `eq-data/index.json`).
- **`.adieqpr` format**: ADI-2 Remote's EQ preset file format, read and written as ADI-2-Pi does.
- **Tauri** (MIT/Apache-2.0) and the Rust crates listed in `src-tauri/Cargo.lock` under their
  own licenses.
- **Trademarks**: RME, ADI-2 and ADI-2 Remote are trademarks of RME Intelligent Audio
  Solutions. This app is not affiliated with or endorsed by RME.
