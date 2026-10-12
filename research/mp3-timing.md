# MP3 timing: how rekordbox positions cues and beat grids in MP3s

rekordbox's cues and beat grids are positions on rekordbox's own decoded timeline. If TuneOx decodes a file to a different timeline, every imported cue and grid line lands early or late. For AIFF, WAV and M4A both timelines agree. For MP3 they agree only if TuneOx handles the file's header frame the way rekordbox does.

## Finding

**When an MP3 starts with a Xing or Info header frame that wasn't written by FFmpeg, rekordbox's timeline is exactly one MP3 frame later than a standard decode** (1,152 samples for MPEG-1 Layer III, 26.12 ms at 44.1 kHz; 576 samples for MPEG-2/2.5). In other words rekordbox plays the header frame as a frame of silence. It does **not** trim the encoder delay/padding described by the LAME tag.

| First frame | rekordbox vs a standard (untrimmed) decode |
| --- | --- |
| No Xing/Info frame | same |
| Xing/Info frame, LAME tag (`LAME3.xx`) | +1 frame |
| Xing/Info frame, no encoder string | +1 frame |
| Xing/Info frame written by FFmpeg (`Lavc…`/`Lavf…`) | same |

A standard decode here means symphonia with gapless trimming off, which skips the header frame and keeps the encoder delay. Trimming the encoder delay (gapless on) moves our timeline a further ~25 ms away from rekordbox's.

## How it was measured

`crates/tuneox-audio/examples/grid_offset.rs` decodes a file (plain, gapless-trimmed, and with TuneOx's lead-in) and cross-correlates the attacks of our full-band peak envelope, at 150 columns per second, with the height channel of rekordbox's own RGB detail waveform (`PWV5`, computed by rekordbox from its decode). A lag of 0 means the two timelines line up. Sub-column precision comes from parabolic interpolation of the correlation peak.

- **Controls** (no encoder delay): 4 AIFF, 3 WAV and 3 M4A tracks from the maintainer's library all came out between −1.5 and −0.1 ms (correlation 0.44–0.91), so the method has a small, constant bias of about −0.5 ms.
- **MP3s**: 50 tracks (two random samples of 10 and 40). Every file with a non-FFmpeg Xing/Info frame (23 in the 40-track sample: 16 Info/LAME, 4 Xing/LAME, 1 Xing with no encoder string, plus 2 whose ID3 tags were large) came out 26.4–28.3 ms early with a plain decode; every file without one, and the 4 FFmpeg-encoded ones, came out within ±1.3 ms.
- **After the fix** (one frame of silence for non-FFmpeg Xing/Info files), all 40 MP3s line up within −4.0…+0.1 ms; 38 within ±1.3 ms, matching the controls.

Raw results: `.re/mp3-alignment-40.txt`, `.re/mp3-alignment-fixed.txt` (not committed).

## What TuneOx does

`tuneox_audio::mp3_lead_in_frames` reads the first Layer III frame after any ID3v2 tag, checks for `Xing`/`Info` at the side-info offset for its MPEG version and channel mode (plus 2 bytes if the frame has a CRC), follows the Xing flags to the LAME-style encoder string, and returns one frame of samples unless that string starts with `Lav`. The decoder writes that much silence before the audio. Positions in TuneOx are therefore on rekordbox's timeline for every file, which is also what Pioneer players expect in exported cues and grids.

## Open

- Why FFmpeg's info frames are treated differently (the frames look structurally the same) isn't known; the rule is empirical.
- Not yet checked: MPEG-2/2.5 files (rare in DJ libraries), VBRI (Fraunhofer) headers, and whether CDJs decode MP3s exactly like rekordbox (they should, since rekordbox's analysis is exported to them unchanged).
