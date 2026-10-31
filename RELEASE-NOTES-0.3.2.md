## Fixed

- **Rear TRS Source** is a Phones 3/4 setting (MIDI table: index 19 on address 9 only; manual:
  "only Phones Out 3/4"). It was shown on the Line Out page and sent to the Line Out address.
- The volume display jumped past a volume limit for a moment when `+` was pressed at the limit;
  volume changes now show the value the unit reports.
- Settings: clearing a Recall field removes the recall volume; before, the change was dropped.
  Edits in a limit profile no longer undo an edit made a moment earlier in the same profile.
- Renaming a saved connection reconnected the unit (and reset an open preset edit). Only a changed
  mode, address, key or demo model reconnects now.
- Preset editor: A (after Listen) also restores the output's EQ preset selection; B re-sends the
  bands as edited since. Both need a unit. Dragging a low-pass or high-pass band in the curve no
  longer changes its gain.
- Preset names that are empty after cleaning (only non-ASCII characters) and renames of an unread
  or empty preset slot are refused instead of being sent.
- `.adieqpr` import: band levels use the scales from `schema.json`; files with `<val …></val>`
  instead of `<val …/>` are read.
- Keyboard shortcuts `M`, `D`, `E` on a page without an output no longer stop the script; an empty
  value typed into a knob is ignored; a dialog opened while another is open closes the first one;
  Escape in a rename dialog cancels it.
- Headphone EQ: empty or out-of-range numbers typed into the options revert or clamp; Bass/Treble
  values are sent only when the result uses them; a stopped worker is terminated; a failed
  measurement download is retried on the next request.
- ADI-2-Pi: a Stop during connect no longer leaves a session open; parameters with keys outside
  the protocol's ranges are ignored.
- USB: an ALSA sequencer buffer overrun re-reads the unit instead of leaving stale values; the
  unit disappearing from the sequencer is reported as unplugged.
- Preset flags word accepted at index 1 (as the unit sends it) and index 2 (as the table lists
  it).
- The command line's reply to an action (such as `set device toggleout 1`) is `sent`.
- Settings are applied on a worker thread; the window no longer blocks while GNOME key bindings
  are applied.
- Device page: sample rate 15 is shown as `none`, other unlisted values as `#n`.

Needs Rust 1.90 or later to build.
