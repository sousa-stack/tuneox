# Feature parity: rekordbox paid features, free in TuneOx

rekordbox 7 has four plans (Free, Core $12/mo, Creative $18/mo, Professional $36/mo) plus a Cloud Option add-on. Some features also unlock while specific Pioneer DJ / AlphaTheta hardware is connected ("Hardware Unlock") or after registering a device ("owner registration"). Checked against rekordbox.com and rekordbox 7.2.19 on 2026-10-12.

**TuneOx has no plans and no license checks. Every feature below is free.** The only limits are things TuneOx can't give away: other companies' subscriptions (streaming services, cloud storage, SoundSwitch) and features that only work through AlphaTheta's own servers.

Milestones: **M1** library + USB export, **M2** performance mode, **M3** Pro DJ Link network, **Later** after that.

## Already free in rekordbox (TuneOx baseline)

Library management and analysis (BPM, key, beatgrid, phrase, vocal detection), USB export in both formats, performance mode by mouse and keyboard, basic STEMS, EDIT mode without saving, vinyl recording, Wi-Fi mobile sync. TuneOx needs all of these to be a credible replacement.

## Library and analysis

| Feature | rekordbox | TuneOx |
| --- | --- | --- |
| Collection Filter, Column View, Playlist Search | Core+, or owner registration / Hardware Unlock | Free, M1 |
| Five screen layouts | Core+ | Free, M1 (layouts are just UI) |
| DUAL PLAYER (two preview players in prep) | Core+, or owner registration / Hardware Unlock | Free, M1 |
| Cloud Analysis (download BPM, grid, waveforms from AlphaTheta's server) | Core+ | Not needed: local analysis on every CPU core should be fast enough. Revisit only if it isn't. |
| Intelligent Cue Creation (learns your cue habits) | Core+ | Free, M1/M2. Rule-based auto cues ("memory cue on first downbeat", "hot cues on phrase changes"), and any LLM can apply cue logic through the MCP server. |
| Collaborative Playlists (up to 5 people) | Professional; needs AlphaTheta cloud | Free, Later, on the sync system below |

## Performance

| Feature | rekordbox | TuneOx |
| --- | --- | --- |
| Control from DJ hardware (controllers, CDJs, mixers in HID/MIDI) | Core+, or Hardware Unlock on ~38 Pioneer/AlphaTheta devices | Free, M2, including non-Pioneer controllers |
| MIDI Learn | Core+, or Hardware Unlock | Free, M2 |
| DVS (timecode vinyl / CDJs) | Core+, or Hardware Unlock on supported mixers and interfaces | Free, Later (after M2) |
| MIX POINT LINK | Core+, or Hardware Unlock (DDJ-FLX10) | Free, M2 |
| Sample Scratch, Merge FX, DJM FX, RMX FX | Core+ (RMX FX: Creative+) | Free, M2 |
| Sampler Sequencer save | Creative+ | Free, M2 |
| Record mixes in performance mode | Core+, or Hardware Unlock | Free, M2 |
| STEMS, all modes and hardware control | Partly free; full control needs a plan or Hardware Unlock | Free, M2, on open source-separation models |
| GROOVE CIRCUIT (drum swap/capture on the drum stem) | Core+, or Hardware Unlock (DDJ-GRV6) | Free, Later (after stems) |

## Editing, video, lighting

| Feature | rekordbox | TuneOx |
| --- | --- | --- |
| Render and save edited tracks (EDIT mode) | Creative+ | Free, Later |
| Video performance (mix video files) | Creative+ | Free, Later |
| Lighting scene editing | Core+ | Free, Later |
| DMX via ENTTEC interfaces | Creative+ | Free, Later. ENTTEC hardware is the user's own; we'd also support Art-Net/sACN. |
| SoundSwitch integration | Creative+, plus a SoundSwitch license | Depends on SoundSwitch allowing it; the license is theirs to sell |

## Cloud and sync

| Feature | rekordbox | TuneOx |
| --- | --- | --- |
| Cloud Library Sync (library + files across computers and mobile) | 20 tracks free; full with Creative+ or Cloud Option | Free, Later. Syncs through storage you already have (any Dropbox/iCloud/Google Drive folder, or S3-compatible), with no TuneOx server or plan. |
| Collection / Playlist Auto Upload | Cloud Option or Creative+ | Free, Later, part of sync |
| Device Library Backup (USB/SD to cloud) | Professional | Free, Later: back up a stick to any folder or storage you choose |
| CloudDirectPlay, Cloud Export, CDP filter / MY TAG sync on players | Plan-limited; runs through AlphaTheta's servers and player firmware | Research only. Players talk to AlphaTheta's cloud, so this works only if that path can be reproduced. |
| Request Catalog + CoBeat audience requests | Plan-dependent; AlphaTheta web app | Free, Later, as a TuneOx-hosted or self-hosted request page |
| Bundled Dropbox storage | Cloud Option / Professional | Not applicable; bring your own storage |

## Things TuneOx can't make free

- **Streaming services** (Apple Music, Spotify, Beatport, SoundCloud, TIDAL): each needs your own subscription and a partner agreement for DJ use. Integrations depend on what those services allow.
- **Cloud storage** (Dropbox, Google Drive, iCloud): your own account.
- **SoundSwitch**: inMusic's license.
- **The rekordbox mobile app's in-app purchases**: separate product, out of scope.

Sources: [rekordbox plans](https://rekordbox.com/en/plan/), [Hardware Unlock](https://rekordbox.com/en/support/hardware-unlock/), [plans FAQ](https://rekordbox.com/en/support/faq/plans-7/), [rekordbox 7 FAQ](https://rekordbox.com/en/support/faq/v7/), [release notes](https://rekordbox.com/en/support/releasenote/).
