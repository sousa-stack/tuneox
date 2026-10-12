# Contributing to TuneOx

Thanks for helping. Start with [docs/status.md](docs/status.md) to see what's done and what's next, and [CLAUDE.md](CLAUDE.md) for conventions. That file is written for AI coding agents but applies to everyone.

## Setup

You need:

- **Rust** (stable). With Homebrew's `rustup`, which is keg-only, add `/opt/homebrew/opt/rustup/bin` to your `PATH`.
- **Bun** for the desktop frontend (`apps/desktop`).
- **Xcode command line tools** on macOS (Tauri needs them).

```bash
cargo test --workspace
cd apps/desktop && bun install && bun run tauri dev
```

### rekordbox database keys (optional)

Importing from rekordbox needs rekordbox's SQLCipher keys at build time. They're publicly known (pyrekordbox documents them) but TuneOx never stores them in the repo. Put them in a `.keys.env` file at the repo root (gitignored):

```
TUNEOX_MASTERDB_KEY=…
TUNEOX_ONELIBRARY_KEY=…
```

Without them everything builds and the tests pass; only importing from a real rekordbox library fails.

### Reverse-engineering tools (optional)

To study rekordbox itself we use [REA](https://github.com/morluto/rea) with Ghidra. See "Reverse engineering rekordbox" in CLAUDE.md, especially the rule that only our own descriptions go into `research/`, never decompiled code or anything from the rekordbox app bundle.

## Making a change

1. Pick something from "Next" or "Known gaps" in `docs/status.md`, or open an issue.
2. New features start as a command in `crates/tuneox-commands`, so they work from the app, the CLI and MCP clients.
3. Add tests. Build fixtures inside the test; never commit real libraries, USB exports or audio.
4. Run `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, and `bun run build` in `apps/desktop`.
5. Update `docs/status.md` (and `research/` or `docs/specs/` if relevant) in the same commit.

## Testing against your own rekordbox library

Point TuneOx at a scratch library so your real one isn't touched:

```bash
cargo run -p tuneox-cli -- --library .re/test-library.db import-rekordbox
cargo run -p tuneox-cli -- --library .re/test-library.db search --bpm-min 124 --bpm-max 128
```

rekordbox's files are only ever read.
