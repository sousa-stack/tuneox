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
- **OneLibrary's `cue` table is empty** on this stick, even though the tracks have cues. On Device Library, cues live only in the ANLZ files (`export.pdb` has no cue table), and OneLibrary players presumably read them from the same ANLZ files. To confirm.

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
- Confirm that OneLibrary players take cues from ANLZ files when the `cue` table is empty.
- What the files outside `rekordbox/` are (`DEVSETTING.DAT`, `MYSETTING*.DAT`, `DJMMYSETTING.DAT`, `djprofile.nxs`, `RBFLTR.DAT`) and which ones a fresh export must create.
