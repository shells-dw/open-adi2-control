Checked against RME's MIDI table version 1.0 (2026-10-29), the first one that covers the
ADI-2/4 Pro EX.

## New

- **ADI-2/4 Pro EX**: recognised by the device revision in its status message (it uses the SE's
  device ID) and shown with its own lists: input Ref Level −35 … +24 dBu, RIAA Mode with 0 dB,
  DC Filter ADC/RME/None, the new Bal. Impedance setting (94 kΩ / 47 kΩ), ADAT 3/4 as a Phones
  3/4 source, and the RIAA functions for the remappable keys. Demo model "ADI-2/4 Pro EX".
- **Unit status** (Settings → Unit) decoded from RME's description of the status message: clock
  source and sample rate, AES and SPDIF inputs (lock, sync, sample rate, 16/24 bit,
  professional, emphasis, non-audio), ADAT, plugged phones, DSD, over current / DC / power-fail
  warnings.
- The page **follows plugged phones**: plugging headphones into Phones 1/2 or 3/4 switches to that
  page (with *Follow the unit* on).
- **Toggle outputs** (Device → Phones), the unit's own toggle function.
- ADI-2 DAC: **Mute Line** (OFF, vs. Phones, Toggle Ph/Line, Toggle plugged).

## Fixed

- ADI-2 Pro and ADI-2/4 with Basic Mode DAC: the output Source list has seven values in DAC mode
  (Auto, AES, SPDIF coax, Optical, Analog, USB 1/2, USB 3/4); it showed the other mode's list.
- ADI-2 DAC: DA filter value 6 shows as Brickwall.
- `open-adi2-control get … | head` no longer prints an error when the reader closes early.

## Not changed

- ADI-2/4 Pro SE / EX Line Out Ref Level stays at five values (+1 … +24 dBu). RME's table 1.0
  still lists the ADI-2 Pro's four for these models, but RME's SE specifications list five
  switchable line output levels.
