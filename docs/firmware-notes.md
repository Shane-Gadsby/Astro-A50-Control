# Tilt auto-off (hard-coded in headset firmware)

Looking down while sitting still turns the headset off in about 15 seconds. This is not a
setting: it is fixed in the headset firmware and cannot be changed through the base
station protocol or Astro Command Center. Notes kept here in case anyone ever tries patching the
firmware.

**Source analysed:** `mimasrx_v39964_6dec2021.bin` (headset/"Rx" firmware) from
`mimas_220420021220_tm-40372_tr-43_rm-39964_rr-43.afw`. The `.afw` is a zip archive with
its `PK` signatures replaced by `AG`; it also holds the base firmware
(`mimastx_v40372`, STM32, loads at `0x0800C000`) and two Avnera radio images.

| | SHA-256 |
|---|---|
| `.afw` | `376edcad419339586878572d5e1619a693f46b795cf113b35806dc079764cd16` |
| `mimasrx_v39964_6dec2021.bin` | `8e7b656b57287ecea24b0266259a8c1b5f0f8a2f21342d0d84944585caa0b85c` |

**How it works.** ARM Thumb (Cortex-M0-class), loads at `0x08005000` (file offset =
address − `0x08005000`). The accelerometer thread (LIS2DH) runs a state machine
`UNINIT → START → ACTIVE → TIMEOUT → SLEEP`. Motion resets the counters. While still, a
5-second timer reads X/Y/Z and classifies the orientation:

- **Upright ("tick V")**: X negative and |Y|, |Z| ≤ 250 (about ±15°). Powers off when
  the V count reaches `auto_power_off_minutes × 12`. The minutes byte is the value set
  with command `0x74`, synced from the base into RAM `0x200004CD`.
- **Tilted ("tick H")**: anything else. Powers off when the H count exceeds 2, i.e. on
  the third still tick, ~15 s.

**Patch points** (address / file offset / bytes as shipped / instruction):

| Address | Offset | Bytes | Instruction | Meaning |
|---|---|---|---|---|
| `0x080071CE` | `0x21CE` | `02 2A` | `cmp r2, #2` | tilted ticks allowed before power-off |
| `0x0800716C` | `0x216C` | `FA 35` | `adds r5, #0xfa` | Y lower bound (−250) |
| `0x08007170` | `0x2170` | `FA 29` | `cmp r1, #0xfa` | Y upper bound (+250) |
| `0x0800717C` | `0x217C` | `FA 32` | `adds r2, #0xfa` | Z lower bound (−250) |
| `0x08007182` | `0x2182` | `FA 23` | `movs r3, #0xfa` | Z upper bound (+250) |
| `0x080071A0` | `0x21A0` | `0C 22` | `movs r2, #0xc` | upright ticks per minute |
| `0x080072A4` | `0x22A4` | `88 13 00 00` | literal `5000` | tick period in ms |

Raising the `cmp r2, #2` immediate (one byte, max 255 ticks ≈ 21 min) is the smallest
change. Widening the ±250 window instead would make "upright" tolerate more tilt.

**Unknowns and risks (why it hasn't been tried):**

- The update path isn't understood: Command Center sends the image to the base, which
  forwards it to the headset (over the dock contacts or radio). Any checksum, signature
  or version checks in the base or headset bootloader haven't been analysed, so a
  modified image may be rejected or brick the headset.
- There is no known recovery path if a flash fails.
- The base firmware contains a `HID_ERROR_*` family and restricted-message checks for
  radio traffic. The firmware-update commands themselves haven't been identified.

A safe first step would be reverse-engineering the update protocol and integrity checks
from Command Center and the base firmware, before touching the headset.
