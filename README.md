# TuneOx

An open-source DJ library manager and rekordbox alternative. [tuneox.com](https://tuneox.com)

TuneOx imports your existing rekordbox library, analyzes your tracks, lets you prep cues and playlists, and exports USB drives that Pioneer DJ / AlphaTheta media players read natively.

> Status: pre-alpha. Nothing to download yet.

## Goals

- **Works on every Pioneer player.** Every export writes both on-device library formats: Device Library (`export.pdb`), read by the CDJ-3000 and older players, and OneLibrary (`exportLibrary.db`), read by the CDJ-3000X, XDJ-AZ, OPUS-QUAD and OMNIS-DUO. One stick works on any player.
- **No lag.** Setting a cue, scrolling a waveform or searching the library responds on the next frame. Disk writes never block the UI.
- **Faster than rekordbox.** Fast startup, instant search across 100k+ tracks, and analysis that uses every CPU core.
- **Your library comes with you.** Import tracks, playlists, cues, beatgrids and analysis from an existing rekordbox install without re-analyzing.
- **Drive it with an AI agent.** Everything TuneOx can do is a command, available from the app, the `tuneox` CLI and an MCP server. Ask Claude (or any MCP client) to build a set, cue a playlist or prep a USB, and watch it happen in the app with undo.
- **Everything is free.** Features that sit behind a rekordbox subscription or hardware unlock are free in TuneOx. No plans, no license checks. See [docs/feature-parity.md](docs/feature-parity.md).

## Roadmap

**Milestone 1: Library + USB export**
- Import a rekordbox 6/7 library (`master.db`)
- Collection, playlists, smart playlists, tags, search
- Analysis: BPM and beatgrid, key, waveforms (preview, detail, 3-band color)
- Cue editor: hot cues, memory cues, loops, colors, comments
- Export to USB: OneLibrary, then Device Library, with incremental sync
- `tuneox` CLI and MCP server covering all of the above

**Milestone 2: Performance mode**
- Decks, mixer, sync, key lock, FX, sampler
- Controller support (DDJ series and MIDI learn)
- Recording

**Milestone 3: Network**
- Pro DJ Link: browse a TuneOx library from CDJs over Ethernet, link export

## Stack

- **Core (Rust):** audio engine, analysis, library database (SQLite), device export
- **Desktop app (Tauri 2):** React + TypeScript UI with GPU-rendered waveforms

See [docs/architecture.md](docs/architecture.md).

## Prior art

TuneOx stands on years of community reverse-engineering work:

- [Deep Symmetry](https://github.com/Deep-Symmetry): crate-digger, beat-link and the [DJ Link ecosystem analysis](https://djl-analysis.deepsymmetry.org/)
- [rekordcrate](https://github.com/holzhaus/rekordcrate): Rust parser for rekordbox device exports
- [pyrekordbox](https://github.com/dylanljones/pyrekordbox): Python access to rekordbox databases, ANLZ files and OneLibrary
- [Mixxx](https://github.com/mixxxdj/mixxx): the open-source DJ application

## License

[GPL-3.0](LICENSE)

TuneOx is not affiliated with or endorsed by AlphaTheta Corporation or Pioneer DJ. rekordbox, CDJ, XDJ and Pioneer DJ are trademarks of their respective owners.
