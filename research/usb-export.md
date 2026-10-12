# USB export formats and player compatibility

## Which player reads which format

From [AlphaTheta's notice on USB devices](https://alphatheta.com/en/information/important-notice-for-customers-using-usb-devices-with-our-dj-equipment/) and the [OneLibrary export guide](https://cdn.rekordbox.com/files/20260318114024/OneLibrary-Compatible-USB-Device-Export_en.pdf) (rekordbox 7.2.12+). Each player reads one format:

| OneLibrary (`exportLibrary.db`) | Device Library (`export.pdb`) |
| --- | --- |
| CDJ-3000X | CDJ-3000 |
| XDJ-AZ | XDJ-XZ, XDJ-RX3, XDJ-RR |
| OPUS-QUAD | CDJ-2000NXS2 and older |
| OMNIS-DUO | XDJ-1000MK2, XDJ-700, CDJ-900NXS2 |

Notes:

- CDJ-3000 firmware 3.30 prioritized OneLibrary when loading; AlphaTheta now lists 3.22 (Device Library) as the latest. Record the firmware version of every CDJ-3000 we test on.
- AlphaTheta says upcoming products will gradually move to OneLibrary.
- rekordbox 7.2.11+ writes both formats on every USB export. 7.2.12+ warns when the two libraries on a stick don't match, with a "Convert from Device Library" action.
- exFAT is only supported by CDJ-3000X, CDJ-3000, XDJ-AZ, OPUS-QUAD, OMNIS-DUO, XDJ-XZ and XDJ-RX3. Everything else needs FAT32 (or HFS+ on Mac-formatted sticks). TuneOx should warn when a stick's filesystem won't work on older players.
- Unverified: whether the XDJ-AZ reads a stick that has only Device Library. The official table says no; no first-hand test yet.

TuneOx writes both formats on every export, so one stick works on any player.

## What a real stick looks like

`ROCKITTRAX6`, one of the user's gig sticks: FAT32, 248 GB, last exported by rekordbox 7.2.9. Database copies are in `.re/fixtures/usb-rockittrax6/`.

```
PIONEER/
  rekordbox/
    export.pdb          3.2 MB   Device Library database
    exportExt.pdb        72 KB   Device Library extension database
    exportLibrary.db    2.1 MB   OneLibrary (SQLCipher)
    RBFLTR.DAT
  USBANLZ/Pxxx/xxxxxxxx/ANLZ0000.DAT|.EXT|.2EX   4,173 / 4,172 / 4,172 files
  Artwork/  CDJ/  MPJ/  extracted/  TrashBox/  .CacheData/
  log/                  player-written *-alog_*.bin
  DEVSETTING.DAT  MYSETTING.DAT  MYSETTING2.DAT  DJMMYSETTING.DAT  djprofile.nxs
Contents/<artist>/<album>/<file>   audio
```

Findings:

- **Both formats share the analysis and audio files.** OneLibrary's `content.analysisDataFilePath` and `content.path` point at the same `/PIONEER/USBANLZ/...` and `/Contents/...` files that Device Library uses. Only the database file differs, so an export writes ANLZ files and copies audio once and then writes two databases.
- **rekordbox 7.2.9 already writes both.** 78 of the 79 OneLibrary playlists also appear by name in `export.pdb`. The track counts are close too: 4,163 OneLibrary tracks against 4,173 analysis files.
- **FAT32 timestamps are unreliable.** `exportLibrary.db` has an mtime of Feb 17, but its own `property.createdDate` says 2026-05-29. Don't judge which library is newer from file dates.
- **OneLibrary's `cue` table is empty** on this stick, even though the tracks have cues. The cues are in the ANLZ files; the reference export below confirms it track by track.

## Reference export: rekordbox 7.2.19, one playlist, four tracks

`TUNEOX-TEST`: a fresh FAT32 stick with one playlist (`10-9-26`) of four 44.1 kHz/16-bit AIFF tracks, exported by rekordbox 7.2.19. Copied to `.re/fixtures/usb-tuneox-test-7.2.19/`, excluding `PIONEER/extracted/gcred.dat` (looks like stored credentials; never read or copy it). This is the export TuneOx's output gets compared against.

```
Contents/<artist>/<album>/<title, truncated>.aiff
PIONEER/rekordbox/export.pdb, exportExt.pdb, exportLibrary.db
PIONEER/USBANLZ/P014/00024E40/ANLZ0000.DAT|.EXT|.2EX     one directory per track
PIONEER/Artwork/00001/a1..a4.jpg, a1_m..a4_m.jpg, b1..b4.jpg, b1_m..b4_m.jpg
PIONEER/MYSETTING.DAT, MYSETTING2.DAT, DJMMYSETTING.DAT, djprofile.nxs
PIONEER/extracted/gcred.dat
```

- A fresh export has no `DEVSETTING.DAT` or `RBFLTR.DAT`, so those come from the players or older rekordbox versions.
- Audio file names are truncated (e.g. `Cinema (Galantis R.aiff`) and keep non-ASCII characters (`B€NZ`).
- Artwork comes in two sets. OneLibrary's `image` table points at `b1..b4.jpg`; presumably Device Library uses `a1..a4.jpg`. Confirm with a PDB parser. `_m` is presumably a smaller size.
- While rekordbox had the stick open, `exportLibrary.db-wal` and `-shm` existed; they were gone after it closed. A copy in a read-only directory couldn't be opened with a plain read-only connection (`SQLITE_READONLY_DIRECTORY`), but `?immutable=1` works. TuneOx's stick reader must handle this, and its writer should finish with no `-wal`/`-shm` left behind.
- OneLibrary schema is identical between rekordbox 7.2.9 and 7.2.19.
- `content.masterDbId` is `djmdDevice.MasterDBID` of the exporting rekordbox, and `content.masterContentId` is the track's `djmdContent.ID`. `analysedBits` = 41 on all four tracks. `cueUpdateCount`, `analysisDataUpdateCount` and `informationUpdateCount` are NULL on a fresh export.

### Cues live in the ANLZ files, not the database

The OneLibrary `cue` table is empty even though the four tracks have 15 cues in rekordbox. The ANLZ files hold them, and the counts match `master.db` exactly:

| Track | master.db cues | `.DAT` PCOB | `.EXT` PCOB | `.EXT` PCO2 |
| --- | --- | --- | --- | --- |
| Cheque | 2 hot | 2 hot | 0 | 2 hot |
| B€NZ | 3 hot | 3 hot | 0 | 3 hot |
| Cinema | 2 hot, 1 memory | 2 hot, 1 memory | 0 | 2 hot, 1 memory |
| Bulletproof | 7 hot (A–G) | 3 hot | 4 hot | 7 hot |

So `.DAT` carries at most three hot cues (A–C, for older players), the `.EXT` PCOB carries the rest, and `.EXT` PCO2 (extended cues with colors and comments) carries all of them.

The Bulletproof hot cues use `djmdCue.Kind` 1, 2, 3, 5, 6, 7, 8 for A–G, skipping 4, which confirms the community mapping (D = 5). Kind 4 turned out to be a slotless loop that rekordbox exports as a memory loop (see `rekordbox-masterdb.md`).

### ANLZ sections per file

| File | Sections |
| --- | --- |
| `.DAT` | `PPTH PVBR PQTZ PWAV PWV2 PCOB PCOB` |
| `.EXT` | `PPTH PWV3 PCOB PCOB PCO2 PCO2 [PQT2] PWV5 PWV4 PSSI` (`PQT2` missing on one track) |
| `.2EX` | `PPTH PWV7 PWV6 PWVC PVDI` |

`PVDI` in `.2EX` isn't in Deep Symmetry's documentation. rekordbox's symbols identify it as vocal detection data; its layout isn't decoded yet. `PWVC` is the 3-band waveform gains (see `waveforms.md`).

## OneLibrary schema (`exportLibrary.db`, `property.dbVersion = 1000`)

22 tables: `album`, `artist`, `category`, `color`, `content`, `cue`, `genre`, `history`, `history_content`, `hotCueBankList`, `hotCueBankList_cue`, `image`, `key`, `label`, `menuItem`, `myTag`, `myTag_content`, `playlist`, `playlist_content`, `property`, `recommendedLike`, `sort`.

Compared with `master.db`, the schema is cleaner. Column names are camelCase, there are no sync bookkeeping columns, and IDs are named per table (`content_id`, `playlist_id`).

- `content`: `title`, `bpmx100`, `length`, `artist_id_artist`, `album_id`, `genre_id`, `key_id`, `color_id`, `image_id`, `rating`, `path`, `fileName`, `fileSize`, `fileType`, `bitrate`, `bitDepth`, `samplingRate`, `analysisDataFilePath`, `analysedBits`, `masterDbId`, `masterContentId`, `cueUpdateCount`, `analysisDataUpdateCount`, …
- `playlist`: `playlist_id`, `sequenceNo`, `name`, `image_id`, `attribute`, `playlist_id_parent`
- `cue`: positions in microseconds (`inUsec`, `outUsec`), plus `in150FramePerSec` (CDJ frames), MPEG frame and byte-offset seek fields (`inMpegFrameNumber`, `inMpegAbs`, `inDecodingStartFramePosition`, `inFileOffsetInBlock`, `inNumberOfSampleInBlock`), and `beatLoopNumerator` / `beatLoopDenominator`.
- `property`: `deviceName`, `dbVersion`, `numberOfContents`, `createdDate`, `backGroundColorType`, `myTagMasterDBID`

## Open questions

- Does the XDJ-AZ accept a Device Library-only stick, a OneLibrary-only stick, or need both?
- The one OneLibrary playlist whose name isn't found in `export.pdb`: probably a non-ASCII name stored as UTF-16. Check once we have a PDB parser.
- Cues: rekordbox 7.2.19 leaves the OneLibrary `cue` table empty and puts every cue in the ANLZ files (see the reference export above). Still to confirm on the XDJ-AZ that cues from a TuneOx export show up.
- The layout of the `.2EX` `PVDI` section, which is rekordbox's vocal detection data (`get_pmai_ex2_vocal_detect_point_num`; see `waveforms.md`).
- What `content.analysedBits = 41` encodes (likely a bitmask of which analyses are present).
- What the files outside `rekordbox/` are (`DEVSETTING.DAT`, `MYSETTING*.DAT`, `DJMMYSETTING.DAT`, `djprofile.nxs`, `RBFLTR.DAT`) and which ones a fresh export must create.
