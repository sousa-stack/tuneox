# rekordbox waveforms in ANLZ files

How rekordbox stores its three waveform styles in the analysis files, what the values mean, and how TuneOx decodes them (`tuneox_pioneer::anlz::AnlzFile::waveform`, `get_waveform`).

Sources, in order of weight:

1. Deep Symmetry's [DJ Link analysis](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html) for the tags and bit layouts.
2. Real files: the TUNEOX-TEST export (rekordbox 7.2.19, four AIFF tracks, `.re/fixtures/usb-tuneox-test-7.2.19/`) and the first 1,500 tracks of the local rekordbox 7.2.19 library (`~/Library/Pioneer/rekordbox/share/PIONEER/USBANLZ/`). Survey scripts and the CSV are in `.re/waveforms/`.
3. The rekordbox 7.2.19 binary (`.re/bin/rekordbox-7.2.19.arm64`). It still has its C++ symbols, so the function names below are rekordbox's own. Addresses are for that thinned arm64 binary. Disassembly excerpts are in `.re/waveforms/` (not committed).

Two findings correct Deep Symmetry's notes: the 3-band entries are **low, mid, high** (not mid, high, low), and `PWVC` is rekordbox's "wave normalize coefficient", the per-band gains of the 3-band waveform.

## Where each waveform lives

| Style | Preview (whole track) | Detail (scrolling) | Not used by TuneOx |
| --- | --- | --- | --- |
| blue | `PWAV` in `.DAT`, 400 columns | `PWV3` in `.EXT`, 150/s | `PWV2` in `.DAT`: 100-column tiny preview (CDJ-900) |
| rgb | `PWV4` in `.EXT`, 1200 columns | `PWV5` in `.EXT`, 150/s | |
| 3-band | `PWV6` in `.2EX`, 1200 columns | `PWV7` in `.2EX`, 150/s | `PWVC` in `.2EX`: band gains (decoded, see below) |

rekordbox's names for them (from `analyzer::CMusicAnalyzeFile::read_pmai_*`): `PWAV` = "wave400pt", `PWV2` = "wave100pt", `PWV3` = "ext zoom wave", `PWV4` = "new whole wave", `PWV5` = "new zoom wave", `PWV6`/`PWV7` = "3bands whole/zoom wave", `PWVC` = "wave normalize coef". `PVDI` (also in `.2EX`) is "vocal detect" data (`get_pmai_ex2_vocal_detect_point_num`); not decoded yet.

A detail waveform has one column per half CDJ frame, 150 per second. The column count is the track length × 150, rounded up: Bulletproof is 306.058 s and has 45,909 columns (45,908.7). Checked on all four fixture tracks (table at the end).

## Section layouts

Every section starts with the fourcc, the header length and the total length (big-endian u32s). All multi-byte values are big-endian. "Seen" values are identical in all 1,504 tracks surveyed unless noted.

### `PWAV`: blue preview

| Offset | Size | Meaning |
| --- | --- | --- |
| 0x0c | u32 | column count (seen: 400) |
| 0x10 | u32 | unknown (seen: `0x00010000`; Deep Symmetry has `0x00100000`) |
| 0x14 | 1 × count | one byte per column |

Byte: height in bits 0–4 (0–31), whiteness in bits 5–7 (0–7). Preview heights top out at 25: in 1,500 tracks the highest column was 25 in 597 tracks and never above. `PWV2` is the same idea with 100 columns and 4-bit heights (max 15).

### `PWV3`: blue detail

| Offset | Size | Meaning |
| --- | --- | --- |
| 0x0c | u32 | entry size (1) |
| 0x10 | u32 | entry count |
| 0x14 | u32 | `0x00960000`: the high half is 150, the column rate |
| 0x18 | count × 1 | entries, same byte layout as `PWAV` (height 0–31, whiteness 0–7) |

### `PWV5`: rgb detail

Header as `PWV3` with entry size 2 and `0x00960305` at 0x14 (high half 150; low half `0x0305` unknown). Each entry is a u16:

| Bits | Meaning |
| --- | --- |
| 15–13 | red, 0–7 |
| 12–10 | green, 0–7 |
| 9–7 | blue, 0–7 |
| 6–2 | height, 0–31 |
| 1–0 | unused, always 0 in the survey |

Red tracks the low band, green the mids, blue the highs: averaged by position within the beat (using the track's `PQTZ` grid), red peaks on the beat and decays slowly (the kick), blue peaks on the off-beat in tracks with off-beat hats. rekordbox computes the color from three band levels in `analyzer::WaveCreator::calcZoomWaveColour(float, float, float)` (0x100d573d4).

### `PWV4`: rgb preview

Header as `PWV3` with entry size 6, entry count 1200 and 0 at 0x14. Each column is six bytes. rekordbox builds them in `analyzer::WaveCreator::createWholeWaveData` (0x100d549fc) from its per-1/150 s analysis data: each preview column covers `duration × 150 / 1200` detail entries and keeps

| Byte | What rekordbox stores | Observed |
| --- | --- | --- |
| 0 | maximum of a full-band amplitude over the column | 0–127 (3 of 1,500 tracks go higher); follows the column's maximum `PWV5` height closely (correlation 0.95–0.97 in all four fixture tracks) |
| 1 | **minimum** of another full-band measure over the column | 0–255, reaches 255 in every track; falls as the music gets louder (correlation −0.41 to −0.88 with height) |
| 2 | maximum of a third full-band measure | 0–127; follows the low band (0.60–0.86) |
| 3 | maximum of the low band, over the column widened by 12 entries each side | 0–127 |
| 4 | maximum of the mid band, over the column widened by 1 entry each side | 0–127 |
| 5 | maximum of the high band over the column | 0–127 |

The band assignment of bytes 3–5 matches Deep Symmetry's notes (byte 3 = bottom third of the spectrum, 4 = middle, 5 = top) and our data: byte 3 correlates best with the `PWV6` low band (0.55–0.87), byte 4 with mid (0.42–0.70), byte 5 with high in three of the four tracks (0.52–0.75; a tie in Bulletproof). What bytes 0–2 measure exactly, and how rekordbox turns the six values into a colored column, is not settled: that's for the rendering research (spec part A). TuneOx passes all six through unchanged.

### `PWV6`: 3-band preview

Header length 20 (no rate field):

| Offset | Size | Meaning |
| --- | --- | --- |
| 0x0c | u32 | entry size (3) |
| 0x10 | u32 | entry count (1200) |
| 0x14 | count × 3 | low, mid, high, one byte each |

rekordbox's own validator (`PCF2::checkAtom2EX`, around 0x101c6c0a0) rejects a `PWV6` unless the header is 0x14 bytes, the entry size 3, the count 1200 and the total length 0xe24.

Values seen: low up to 75, mid up to 97, high up to 106. They are built separately from the detail (`create3BandsWholeWaveData`, 0x100d55210), with their own per-band weighting, and come out at roughly half the detail's average level, because the preview draws the three bands stacked.

### `PWV7`: 3-band detail

Header as `PWV3` with entry size 3 and `0x00960000` at 0x14. Each entry is low, mid, high, one byte each, **0–127**: no value above 127 in any band of 1,500 tracks (the formula below could produce 128 in an extreme case).

#### Band order

Deep Symmetry documents the order as mid, high, low. rekordbox and the data say low, mid, high:

- rekordbox's CDJ waveform reader copies the three bytes as they are (`djplay::WaveCDJ::get3BandItem`, 0x1002785bc), and its accessors read byte 0 in `getFreqLow`, byte 1 in `getFreqMid` and byte 2 in `getFreqHigh` (`djplay::WaveDataCDJ`, 0x1022182b0–0x102218318).
- Averaged by position in the beat (Bulletproof, Cheque, B€NZ), byte 0 rises on the beat and decays over half a beat like a kick drum, byte 2 is near zero except for sharp peaks on the beat and the off-beat (hats), byte 1 sits in between. Over whole tracks (Bulletproof, Cheque, Cinema), byte 0 correlates best with `PWV5` red × height (0.84 in each), byte 1 with green (0.77–0.84), byte 2 with blue (0.75–0.76).

#### How rekordbox computes the levels

`analyzer::WaveCreator::create3BandsZoomWaveData` (0x100d55fbc) turns three internal 16-bit band levels into the stored bytes:

1. **Smoothing.** Each band follows its input with an instant attack and an exponential release: when a column is louder than the running value, the running value jumps to it; otherwise it moves towards the new value, keeping 99 % (low), 98 % (mid) or 97 % (high) of the difference each column. The follower starts 300, 200 and 100 columns before the first output column respectively.
2. **Gain.** Each band is multiplied by its gain from `PWVC`.
3. **Scale and curve.** Low and mid become `gain × level / 256` (0–127). High goes through a raised-cosine curve instead, `gain × 64 × (1 − cos(π × level / 32768))`, which flattens quiet hiss and keeps the hats.

So the stored levels are final: smoothed, balanced and curved. A renderer can draw them as they are.

### `PWVC`: 3-band gains

| Offset | Size | Meaning |
| --- | --- | --- |
| 0x0c | 2 bytes | unknown, always 0 (header length is 14) |
| 0x0e | 3 × u16 | low, mid, high gain × 100 |

rekordbox's validator requires a header of 0x0e bytes and a total of 0x14 (`PCF2::checkAtom2EX`, around 0x101c6c2b4). The player reads the three values as `u16 × 0.01`, defaulting to 1.0 when they are missing (`djplay::DjplayDB::getWaveNormalizeCoef`, 0x1001fece4).

How they're chosen (`create3BandsWholeWaveData`, around 0x100d55c40): rekordbox weighs the bands against each other from their totals over the track, caps each gain so the band's loudest point stays inside the range (32768 / peak), then clamps it to **0.80–3.00** for low and mid and **0.80–5.00** for high, and stores it × 100. Survey of 1,500 tracks:

| Band | Range seen | Most common |
| --- | --- | --- |
| low | 80–300 | 80 (631 tracks, the floor: bass is usually the loudest band) |
| mid | 80–300 | 100 (363) |
| high | 80–500 | 100 (260) |

The data confirms the gain is baked into `PWV7`: a band's stored values never exceed 128 × gain (rounded down), the largest a 16-bit level can give, and with gains below 1 they usually reach it. Low gain 0.80 → maximum 102 in 286 of 631 tracks (the rest 99–101); 0.81 → 103; 0.82 → 104; 0.84 → 107; 0.86 → 110; 0.88 → 112; 0.91 → 116. Cheque's mid gain of 0.83 gives a maximum of 106.

**Does rendering need it?** Not to draw the stored levels, which already include it. But rekordbox's 3-band views do receive the gains (`djplay::WaveView::createObject(…, WaveNormalizeCoef)`, 0x1021944b4) and multiply band values by them while building the picture, both in the view drawn from its own analysis data (`create3BandsWaveInfo`, 0x102192b48, which repeats the smoothing, gain and curve above at draw time) and in the CDJ-style view drawn from these files (`draw3BandsWaveCDJ`, 0x102190bf0). Whether that second path ends up applying the gain twice, or the gains are 1.0 there, isn't settled. TuneOx returns the gains with every 3-band waveform (`band_gains`) so the renderer can follow whatever the rendering research finds.

## How rekordbox draws them

What the binary shows so far; spec part A owns the full picture.

- **Blue.** rekordbox converts the 3-bit whiteness of a blue detail column into a color through an eight-entry table and then draws it like an RGB column (`analyzer::CAnalyzerIF::load_blueZoomWave`, 0x10161ffe4; table `get_cdjZoomWaveColor::cdjZoomWaveRGBTable` at 0x1037e588c):

  | Whiteness | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | Color | `#083C8C` | `#085DBD` | `#006DFF` | `#08AAF7` | `#52B6F7` | `#73AEFF` | `#73AEFF` | `#CEE7FF` |

  It keeps only the top 3 bits of each channel of these colors (e.g. whiteness 0 becomes red 0, green 1, blue 4 in `PWV5` terms).
- **RGB detail.** Height and the 3-bit color per column. How rekordbox widens 3-bit channels to screen colors isn't confirmed.
- **3-band.** The CDJ-style 3-band routine (`draw3BandsWaveCDJ`) sets up seven colors: `#0055E1` (blue), `#FFA600` (amber), `#FFFFFF` (white), `#B4690A` (brown), `#D2DCFA`, `#FFF0D7`, `#F5EBD7`. Deep Symmetry's description fits the first four: lows dark blue, mids amber, highs white, brown where low and mid overlap; the preview stacks the bands, the detail draws them on one axis with high last. The roles of the last three are unconfirmed (probably overlaps involving the highs).

## TuneOx's records

`tuneox_core::Waveform` holds one fixed-size record per column. Every value is a byte where 255 is the full scale of the source field, so narrower Pioneer fields are widened with `round(v × 255 / max)` (`anlz::widen`); `anlz::narrow` reverses it exactly, which an exporter needs.

| Style | Resolution | Bytes | Record | Source fields |
| --- | --- | --- | --- | --- |
| `blue` | preview, detail | 2 | height, whiteness | 5-bit height, 3-bit whiteness |
| `rgb` | detail | 4 | red, green, blue, height | 3-bit colors, 5-bit height |
| `rgb` | preview | 6 | `PWV4` bytes 0–5, unchanged | see the `PWV4` table |
| `three_band` | preview, detail | 3 | low, mid, high | 0–127 each (above 127 saturates at 255) |

3-band waveforms also carry `band_gains` (`PWVC` / 100) when the file has them. Detail waveforms carry `columns_per_second`, read from the high half of the header word at 0x14 (150; falls back to 150 if zero); previews have none.

Decoding errors: a missing section is `None` (the command answers `NotFound`); a header that's too short, an unexpected entry size, or a count larger than the body is `Error::Corrupt` with the byte offset.

## Verification against real files

The four TUNEOX-TEST tracks (rekordbox 7.2.19 export), decoded with `cargo run -p tuneox-pioneer --example anlz_dump`:

| Track | Audio length | Detail columns (blue = rgb = 3-band) | length × 150 | Gains low / mid / high | Max preview height (`PWAV`, of 31) |
| --- | --- | --- | --- | --- | --- |
| Bulletproof (`P073/0001670F`) | 306.058 s | 45,909 | 45,908.7 | 0.83 / 1.13 / 1.03 | 25 |
| Cheque (`P014/00024E40`) | 247.869 s | 37,181 | 37,180.3 | 0.80 / 0.83 / 1.01 | 25 |
| B€NZ (`P02F/0000E4E5`) | 245.039 s | 36,756 | 36,755.9 | 0.80 / 1.00 / 1.16 | 25 |
| Cinema (`P039/000273B3`) | 196.374 s | 29,457 | 29,456.1 | 0.99 / 0.89 / 1.04 | 25 |

Every preview has 400 (`PWAV`) or 1200 (`PWV4`, `PWV6`) columns. Sample decoded values agree across styles: Bulletproof's first three detail columns are silent (height 0); its middle detail column is red 255, green 0, blue 36, height 49 in RGB, and low 124, mid 40, high 16 in 3-band (TuneOx's 0–255 values), so both styles see a bass-heavy moment.

Other checks:

- **Survey of 1,500 local tracks**: every header constant above is the same in all of them; `PWV5`'s two low bits are always 0; `PWV3`, `PWV5` and `PWV7` have the same column count in 1,499 tracks, and one track's `PWV3` is a column shorter. Don't assume the three styles line up exactly; use each waveform's own count.
- **Local library vs USB export**: for Bulletproof, every waveform section (`PWAV`, `PWV2`–`PWV7`, `PWVC`) is byte-for-byte the same in the local analysis and on the stick. rekordbox copies them on export, so a TuneOx exporter can too.
- **Speed**: `get_waveform` for Bulletproof (306 s), release build, averaged over 50 calls: detail 0.15 ms (blue), 0.21 ms (rgb), 0.17 ms (3-band); previews 0.03 ms. The first call, which reads the file from disk, took up to 0.7 ms. Decoding alone is 60–110 µs per detail waveform.

## Open questions

- How rekordbox draws the RGB preview: which of the six `PWV4` values give the height(s) and color, and what bytes 0–2 measure.
- How rekordbox widens 3-bit colors for display, and the exact 3-band overlap colors and blending.
- Whether the CDJ-style 3-band view applies the `PWVC` gains on top of the stored levels.
- `PVDI` (vocal detection) layout; the low half (`0x0305`) of `PWV5`'s rate word.
