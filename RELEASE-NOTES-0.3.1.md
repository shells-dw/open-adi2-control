## Fixed

- Loudness **Low Vol Ref** was read and set in 0.1 dB steps. The unit uses 0.5 dB steps (RME's
  example dump in the MIDI table), so the knob showed a fifth of the value and set values outside
  the unit's range.
- Volume keys with *The output turned last*: a volume changed in the window now counts as turned.
- Command line: `set … nan` no longer sets 0 dB; parameter numbers outside the protocol's ranges
  and setup or preset numbers outside 1–9 / 1–20 are refused instead of being sent.
- `.adieqpr` import: values outside the unit's ranges are brought into range; XML entities such
  as `&apos;` in names are decoded.
- A settings file that can't be read is kept as `config.json.bad` instead of being overwritten.
- Settings changed from the tray menu while a change from the window was being applied (GNOME
  key bindings take a moment) were lost.
- ADI-2-Pi: commands queued while a connection broke are not sent on the next one.
- USB: a MIDI connection removed by another program is made again; an answer to an earlier probe
  identifies the unit at once.
- Without `XDG_RUNTIME_DIR` the command socket is created under `~/.cache/open-adi2-control/`
  instead of the shared temporary directory.

RME's example messages from the MIDI table (full dump, setup names, EQ preset answers, status)
are now part of the test suite.
