# Research: why the A50 turns off when you look down

This is a record of research into the A50's tilt auto-off, and why it stopped short of
changing anything. It isn't a guide, and nothing here has been tested on a headset.

> [!CAUTION]
> Don't flash a modified image based on these notes. Whether the headset would accept one
> is unknown, and there's no known way to recover it if a flash fails.

## Summary

- **Question:** can the A50 be stopped from turning off when it's tilted and still?
- **Answer:** not through any setting. The behaviour is fixed in the headset firmware.
  Changing it would mean patching that firmware.
- **Outcome:** the logic and the values that control it were found, but no patch was made.
  Getting a change onto the headset safely is an unsolved problem, and the risk is
  [losing the headset](#why-it-wasnt-pursued).

## The behaviour

Looking down while sitting still turns the headset off in about 15 seconds. This isn't a
setting: it can't be changed through the base station protocol or Astro Command Center.
The separate auto power-off setting (10–30 minutes) only applies while the headset is
upright.

## Method

This was static analysis of the firmware file. Nothing was modified, flashed or run on
the hardware.

No Astro firmware or software is included in this repository. The file names and hashes
below only identify the versions studied.

**Source analysed:** `mimasrx_v39964_6dec2021.bin` (headset/"Rx" firmware) from
`mimas_220420021220_tm-40372_tr-43_rm-39964_rr-43.afw`. The `.afw` is a zip archive with
its `PK` signatures replaced by `AG`; it also holds the base firmware
(`mimastx_v40372`, STM32, loads at `0x0800C000`) and two Avnera radio images.

| | SHA-256 |
|---|---|
| `.afw` | `376edcad419339586878572d5e1619a693f46b795cf113b35806dc079764cd16` |
| `mimasrx_v39964_6dec2021.bin` | `8e7b656b57287ecea24b0266259a8c1b5f0f8a2f21342d0d84944585caa0b85c` |

## Findings

The headset firmware is ARM Thumb (Cortex-M0-class) and loads at `0x08005000` (file offset
= address − `0x08005000`). The accelerometer thread (LIS2DH) runs a state machine
`UNINIT → START → ACTIVE → TIMEOUT → SLEEP`. Motion resets the counters. While still, a
5-second timer reads X/Y/Z and classifies the orientation:

- **Upright ("tick V")**: X negative and |Y|, |Z| ≤ 250 (about ±15°). Powers off when
  the V count reaches `auto_power_off_minutes × 12`. The minutes byte is the value set
  with command `0x74`, synced from the base into RAM `0x200004CD`.
- **Tilted ("tick H")**: anything else. Powers off when the H count exceeds 2, i.e. on
  the third still tick, ~15 s.

So the 15 seconds is three 5-second ticks, and the tilt limit is the ±250 window.

### Where the values live

These are the locations that control the behaviour, as shipped in this firmware version:

| Address | Offset | Bytes | Instruction | Meaning |
|---|---|---|---|---|
| `0x080071CE` | `0x21CE` | `02 2A` | `cmp r2, #2` | tilted ticks allowed before power-off |
| `0x0800716C` | `0x216C` | `FA 35` | `adds r5, #0xfa` | Y lower bound (−250) |
| `0x08007170` | `0x2170` | `FA 29` | `cmp r1, #0xfa` | Y upper bound (+250) |
| `0x0800717C` | `0x217C` | `FA 32` | `adds r2, #0xfa` | Z lower bound (−250) |
| `0x08007182` | `0x2182` | `FA 23` | `movs r3, #0xfa` | Z upper bound (+250) |
| `0x080071A0` | `0x21A0` | `0C 22` | `movs r2, #0xc` | upright ticks per minute |
| `0x080072A4` | `0x22A4` | `88 13 00 00` | literal `5000` | tick period in ms |

In principle, raising the `cmp r2, #2` immediate (one byte, max 255 ticks ≈ 21 min) would
be the smallest change. Widening the ±250 window instead would make "upright" tolerate
more tilt.

## Why it wasn't pursued

Finding the values turned out to be the easy part. The blockers are all about getting a
changed image onto the headset without breaking it:

- **The update path isn't understood.** Command Center sends the image to the base, which
  forwards it to the headset (over the dock contacts or radio). Any checksum, signature
  or version checks in the base or headset bootloader haven't been analysed, so a
  modified image may be rejected or brick the headset.
- **There's no known recovery path** if a flash fails.
- **The firmware-update commands haven't been identified.** The base firmware contains a
  `HID_ERROR_*` family and restricted-message checks for radio traffic, but the commands
  themselves remain unknown.

The possible gain is a headset that stays on while you look down. The possible cost is a
headset that never turns on again, so the work stopped here.

## If this is ever picked up

The safe next step doesn't involve touching the headset: reverse-engineer the update
protocol and integrity checks from Command Center and the base firmware first. Only once
those are understood, and a recovery path is known, would a patch be worth considering.
