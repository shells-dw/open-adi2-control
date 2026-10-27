## Fixed

- ADI-2/4 Pro SE: Line Out Ref Level has five values (+1, +7, +13, +19, +24 dBu). 0.2.0 and
  earlier used the ADI-2 Pro's four (+4, +13, +19, +24 dBu), so the level was shown and set one
  step off. The order is taken as ascending, like the SE's input list in RME's table; not yet
  checked on an SE.
- `open-adi2-control get line ref` printed a phones level name instead of the line level.

## Changed

- Device ID 0x73 is shown as "ADI-2/4 Pro SE / Pro EX" (the ADI-2/4 Pro EX is expected to use the
  SE's ID).
- A select value the unit reports that RME's MIDI table doesn't list (for example settings the
  ADI-2/4 Pro EX added) is shown as `#n` instead of `(n)` or an empty field; the command line
  prints it as `#n` and accepts `#n` for `set`.
