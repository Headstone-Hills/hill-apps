# M-VAVE FM-1 MIDI: what the vendor documents, and what is still unverified

Collected 2026-11-08. The section "Verified on the owner's unit" below is what has been checked by ear on a real FM-1 running firmware **V16**; everything else is from documents only.
**Do not put CC numbers into code from this file until they are verified** (HANDOFF rule: do not invent CC numbers).

## Sources

| What | Where |
|---|---|
| Official download page (FM-1 entry: firmware V16, dated 2026_10_21; lists "MIDI CONTROL EN/CN", the firmware and the release note) | <https://www.m-vave.com/download.html> |
| **MIDI guide, English** (`FM-1 MIDI EN.docx`, 67 KB) | <https://yms-file-store.oss-cn-hongkong.aliyuncs.com/software/releaseNote/firmware/FM-1%20MIDI%20EN.docx> |
| MIDI guide, Chinese | same folder, `FM-1%20MIDI%20CN.docx` |
| **Release note** (all versions, plain text) | <https://yms-file-store.oss-cn-hongkong.aliyuncs.com/software/releaseNote/firmware/FM-1.txt> |
| Announcement of V15 and "a new FM-1 MIDI CC Control Parameter Guide" | <https://www.matrixsynth.com/2026/07/m-vave-fm-1-v15-firmware-update-new.html> |
| Owner forum thread (users testing CC over USB, June-July 2026) | <https://www.elektronauts.com/t/m-vave-fm-1/252170?page=2> (and page 5) |

The `.docx` files are the vendor's; they are not copied into this repo. The tables below are a transcription of the facts.

## What the English MIDI guide says

**Channels.** The note channel (`midich`) defaults to All / Omni. The FX channel (`effectch`) defaults to **MIDI channel 2**. System Real-Time has no channel.
Warning in the guide: CC 1 (mod wheel) sent on the FX channel acts as an FX parameter (Filter Type), not mod wheel; send mod wheel on another channel. Pitch bend is unaffected.

**Note channel**

| Message | Byte | Data | Response |
|---|---|---|---|
| Note On | 9n | note, velocity 1-127 | Sound (velocity 0 = Note Off) |
| Note Off | 8n | note | Key release |
| Program Change | Cn | 0-127 | Voice 001-128 |
| Pitch Bend | En | 14-bit | Pitch bend |
| Aftertouch | Dn | 0-127 | Channel aftertouch |
| Mod Wheel | Bn | CC 1 | Modulation |
| Sustain | Bn | CC 64 | Sustain (release on off) |

**FX channel, CC 0-23** (this is the table in the guide as downloaded; see the conflict below)

| CC | Effect | Parameter | Range |
|---|---|---|---|
| 0 | Filter | Switch | 0 = off, >=1 = on |
| 1 | Filter | Type | 0-2 (LPF / BPF / HPF) |
| 2 | Filter | Cutoff | 0-107 |
| 3 | Filter | Q | 0-10 |
| 4 | Reverb | Switch | 0 / >=1 |
| 5 | Reverb | Type | 0-2 (Room / Hall / Plate) |
| 6 | Reverb | Decay | 0-100 |
| 7 | Reverb | Mix | 0-100 |
| 8 | Delay | Switch | 0 / >=1 |
| 9 | Delay | Decay | 0-100 |
| 10 | Delay | Rate | 0-100 |
| 11 | Delay | Mix | 0-100 |
| 12 | Distortion | Switch | 0 / >=1 |
| 13 | Distortion | Gain | 0-100 |
| 14 | Distortion | Tone | 0-100 |
| 15 | Distortion | Level | 0-100 |
| 16 | Chorus | Switch | 0 / >=1 |
| 17 | Chorus | Freq | 0-100 |
| 18 | Chorus | Depth | 0-100 |
| 19 | Chorus | Mix | 0-100 |
| 20 | Phaser | Switch | 0 / >=1 |
| 21 | Phaser | Freq | 0-100 |
| 22 | Phaser | Depth | 0-100 |
| 23 | Phaser | Mix | 0-100 |

**System Real-Time:** F8 clock (follows external BPM), FA start (from step 0), FB continue (same as start, no resume), FC stop.

**SysEx:** `F0 43 10 pp qq vv F7` writes one parameter: `param = pp*128 + qq` (0-155), value `vv`. The parameter numbers are **not** listed in the guide.

## What the release note says (and why it conflicts)

- **V16 (2026-10-20, firmware 016):** each Voice has its own MIDI receive channel (voices can share a channel to layer, or use separate channels). CC11 (Expression) works on matching channels, 0-127; CC121 resets Expression to 127 for those voices only. **CC7 is master volume.** CC7, CC11 and sustain routing were fixed for channels that overlap the Effect Ch.
  ARP accepts external MIDI notes as chord input. **FX are controlled over MIDI by continuous CC 70-93 on the Effect Ch channel** (Filter, Reverb, Delay, Distortion, Chorus, Phaser). **The old CC 0-23 FX mapping was removed**; CC1, CC2 and CC4 remain sound-modulation controls, so existing controllers need the new numbers.
  Notes 0-26 on the Effect Ch light the 27 keyboard LEDs (zero-velocity Note On or Note Off clears them; no sound). Bank SysEx import/export (DX7 `.syx`), a USB Loopback change, and PLAY/STOP held = Panic.
- **V15 (2026-07-30):** Note On with velocity 0 is treated as Note Off. CC responses adjust the FX parameter scale.
- **V13 (2026-07-03):** MIDI channel selection renamed from "note ch" to "MIDI ch" (default ALL); **Program Change 0-127 selects sounds**; ARP and sequencer sync to BPM.

**The conflict:** the guide on the download page is attached to the V16 entry but still documents the FX controls as **CC 0-23**, which the V16 release note says were removed in favour of **CC 70-93**. Either the guide was not updated for V16 or the release note is describing something the firmware did not ship. The release note gives only the range (70-93, six effects, 24 controls); it does **not** say which number is which. It is tempting to assume "old number + 70" (CC 0 -> 70 ... CC 23 -> 93), but that is a guess and is not recorded as fact here.

## Other facts from the forum (users, not the vendor)

- sm7x7 (2026-06-26) tried CC 7, 10, 11, 91 and 93 and reported that notes worked but the CCs did not. mksl (2026-06-28) suggested the cause: the FX channel is a separate global setting from the note channel, so CCs sent on the note channel do nothing.
- mksl (2026-07-09): "quick test over usb midi says no" to CC; (2026-07-12) says firmware v13 added Program Change, not CC. Both predate V16 and the FX-channel explanation above, so treat as unreliable.
- A third-party browser tool for FM-1 SysEx bank management exists (<https://grandsummoner.github.io/fm-1-helper/FM-1%20Helper.html>). Not evaluated.

## Verified on the owner's unit (firmware V16, 2026-11-08, by ear, one listener)

Method: `fm1tool cc N V --ch C` then `fm1tool note 60`, compared with a plain note. "Duller" = filter closed.

| Test | Result |
|---|---|
| Old guide numbers, CC 0 = on and CC 2 = 5, on channel 2 and on all 16 channels | **no audible change** (the CC 0-23 table does not apply to V16) |
| New numbers, CC 70 = 127 and CC 72 = 5, on all 16 channels at once | **duller** |
| Same, one channel at a time, 1-16 | **only channel 2 is duller** (the Effect Ch default) |
| CC 8 / CC 78 = 127 on channel 2 (delay switch guesses), then a note | no audible change either way; not informative (a delay with Mix at 0 is silent) |

So: **on V16, FX controls are CC 70-93 on channel 2, and the old CC 0-23 do nothing.** Notes and program change use the note channel (the FM-1 answers note-on on channel 1 and Omni).
Not yet checked: which of CC 70 (filter switch?) and CC 72 (cutoff?) caused the change, and what the other 22 numbers do. The "old number + 70" order (CC 70 Filter switch, 71 Type, 72 Cutoff, 73 Q, 74 Reverb switch ... 93 Phaser Mix) is consistent with the two that worked but is **still a guess for the rest**. The unit keeps its FX state: the tests left the filter switch off and cutoff at 127.

## What this means for HillDAW

1. **FM-1 CC control must go to the FX channel (default 2), not channel 1.** `fm1tool cc N V` defaults to channel 1; use `--ch 2`. Notes and program change go to the note channel.
2. **Program Change browsing is documented** (0-127 -> Voice 001-128, firmware >= V13). That is the safest first target for the patch browser.
3. **The handoff's "treat it as monotimbral" assumption may be wrong** for V16: per-Voice receive channels imply multitimbral use. Unverified.
4. **The unit's own sound can probably be silenced for sampling** by routing it to a channel nothing listens on, but nothing here says so; test.
5. Firmware version of the owner's unit is unknown. It decides which CC table applies.

## Remaining checks (one command at a time; listen or watch the FM-1 display)

First learn the firmware version (FM-1 global settings screen, or the version the FM-1 Helper tool / firmware updater shows).

```
fm1tool pc 5 --ch 1          # note channel: should change the sound (program change)
fm1tool cc 0 0 --ch 2        # old table: Filter switch off  (V16 should ignore it if the old map is gone)
fm1tool cc 0 1 --ch 2
fm1tool cc 70 0 --ch 2       # V16 range: expect a Filter switch or similar; listen for any change
fm1tool cc 70 1 --ch 2
fm1tool cc 7 40 --ch 1       # CC7 = master volume per release note; the note says it was fixed for channels
fm1tool cc 7 127 --ch 1      #   that overlap the Effect Ch, so try it on both channels
fm1tool cc 7 40 --ch 2
fm1tool cc 7 127 --ch 2
```

Record for each: firmware, channel, CC, value, what changed. Then write the confirmed rows into a data file (not code) for the patch/FX browser.
