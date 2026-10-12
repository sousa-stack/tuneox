# TuneOx

Open-source rekordbox alternative: a DJ library manager that imports a rekordbox library, analyzes and preps tracks, previews mixes on two decks, and exports USB sticks every Pioneer DJ / AlphaTheta player can read. Rust core + Tauri 2 desktop app. GPL-3.0. Site: tuneox.com.

**Start here:** [docs/status.md](docs/status.md) says what works, what's in progress and what's next. [docs/architecture.md](docs/architecture.md) explains how the pieces fit. Read both before structural changes.

## Keep the docs current

Every change updates the docs it affects **in the same commit**:

- [docs/status.md](docs/status.md): move items between Done / In progress / Next, add dated entries to the decision log, record new known gaps. This file is how the next person (or agent) knows where things stand.
- `research/<topic>.md`: anything learned about rekordbox or Pioneer formats and behavior.
- `docs/specs/<feature>.md`: the contract for a feature being built by more than one person or agent. Update it if the implementation has to deviate.
- This file: when a convention, command or workflow changes.

## What decides whether a change is done

- **Exports work on every Pioneer/AlphaTheta player**, in both Device Library (`export.pdb`) and OneLibrary (`exportLibrary.db`) formats. See [research/usb-export.md](research/usb-export.md).
- **No lag.** Cue edits, waveform scroll/zoom, search and deck controls respond on the next frame. The UI never awaits a disk write or a core round trip on an interactive path. See "Latency rules" in the architecture doc.
- **Faster than rekordbox** at startup, search and analysis.
- **Every feature is a command first.** It lives in `crates/tuneox-commands` and is reachable from the desktop app, the `tuneox` CLI and the MCP server. No UI-only features. See "Automation" in the architecture doc.
- **Everything is free.** Features that are paid in rekordbox are free in TuneOx. Never add plan gating or license checks. See [docs/feature-parity.md](docs/feature-parity.md).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass, and `bun run build` passes in `apps/desktop` (CI runs all of these).

## Repo map

| Path | What it is |
| --- | --- |
| `crates/tuneox-core` | Domain types: `Track`, `Cue`, `BeatGrid`, `Playlist`, `MusicalKey`. No I/O. Positions are integer microseconds (`*_us`). |
| `crates/tuneox-pioneer` | rekordbox/Pioneer formats: `master.db` reader, ANLZ analysis files, DB keys (`build.rs`), research examples. |
| `crates/tuneox-library` | TuneOx's own SQLite library: schema (`schema.sql`), FTS5 search, transactional imports. |
| `crates/tuneox-commands` | The command registry. Every user-facing operation, with JSON schemas and an effect class. |
| `crates/tuneox-mcp` | MCP server over the registry (stdio; modern 2026-07-28 and legacy protocol versions). |
| `apps/cli` | The `tuneox` binary: subcommands, `run <command> <json>`, `commands`, `mcp`. |
| `apps/desktop` | Tauri app. `src-tauri/src/lib.rs` exposes one `run_command` bridge; `src/` is React + TypeScript. |
| `research/` | What we learned about rekordbox, in our own words, with evidence. |
| `docs/` | Architecture, status, feature parity, specs. |
| `.re/` | Gitignored scratch space: raw reverse-engineering output, copies of real libraries and USB exports. |

## Commands

```bash
cargo build                                   # whole workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p tuneox-cli -- --help             # CLI
cargo run -p tuneox-cli -- commands --json    # command catalog with JSON schemas (= MCP tools)
cd apps/desktop && bun install && bun run tauri dev            # desktop app, dev mode
cd apps/desktop && bun run tauri build --bundles app           # → target/release/bundle/macos/TuneOx.app
```

The desktop app, CLI and MCP server open the library at `$TUNEOX_LIBRARY` if set, else `~/Library/Application Support/TuneOx/library.db` (macOS) or the OS data folder elsewhere. **When testing against real data, point `TUNEOX_LIBRARY` at a scratch file under `.re/`** so nobody's real TuneOx library is touched:

```bash
cargo run -p tuneox-cli -- --library .re/test-library.db import-rekordbox --dry-run
cd apps/desktop && TUNEOX_LIBRARY=$PWD/../../.re/test-library.db bun run tauri dev
```

Research tools:

```bash
cargo run -p tuneox-pioneer --example rbdb -- masterdb path/to/master.db [SQL]          # rekordbox library DB
cargo run -p tuneox-pioneer --example rbdb -- onelibrary path/to/exportLibrary.db [SQL] # OneLibrary USB DB
cargo run -p tuneox-pioneer --example anlz_dump -- path/to/ANLZ0000.EXT                # analysis files
```

## Conventions

- **Commands.** A new feature adds a `Command` impl in `tuneox-commands` (typed input/output deriving `JsonSchema`, a description written for people and LLMs, an `Effect`), registered in `registry![…]`. The UI calls it through `api.ts` → `run_command`; don't add bespoke Tauri commands. Bulk binary data (e.g. waveforms) goes in command output as base64, not as JSON number arrays.
- **Units.** Positions inside a track are `i64` microseconds; per-beat tempos are BPM × 100; library timestamps are unix milliseconds.
- **Errors.** Command failures are `tuneox_commands::Error`; the MCP server returns them as tool errors (`isError`) so models can self-correct.
- **Tests.** New behavior gets tests. Fixtures are built by our own code inside the test (see `crates/tuneox-commands/src/import_rekordbox/tests.rs`, which builds a fake encrypted `master.db` and ANLZ file). Never commit real user databases, exports or audio.
- **Style.** Match the surrounding code. Comments explain why, not what.
- **Commits** say what changed and why; keep docs in the same commit.

## Database keys

rekordbox's databases are SQLCipher-encrypted with fixed keys. TuneOx never stores them in the repo: `crates/tuneox-pioneer/build.rs` compiles them in from the `TUNEOX_MASTERDB_KEY` / `TUNEOX_ONELIBRARY_KEY` environment variables (GitHub Actions secrets in CI) or a gitignored `.keys.env` at the repo root. Without them everything builds and tests pass, but importing from rekordbox fails with a clear error. **Never write key values into source, docs, tests or commit messages.**

## Reverse engineering rekordbox

We use [REA](https://github.com/morluto/rea) (an MCP server and CLI, `npx rea-agents`) with Ghidra to study rekordbox. rekordbox is a JUCE C++ app with RTTI class names intact, so searching symbols by class name is the fastest way in.

1. Check prior work first: Deep Symmetry's [DJ Link analysis](https://djl-analysis.deepsymmetry.org/), rekordcrate, pyrekordbox, beat-link. Only reverse engineer what they don't cover or what changed in rekordbox 7.
2. Raw output (pseudocode, disassembly, Ghidra projects, dumps) goes in `.re/`.
3. Write what you learned **in your own words** in `research/<topic>.md`: structures, field meanings, byte layouts, constants, behavior, and the evidence (function name/address, sample file).
4. Implement from the research notes.

Never commit decompiled Pioneer code, Pioneer binaries or libraries, or anything from the rekordbox app bundle (skins, images, models, MIDI mapping files, demo tracks, sample packs). Don't copy code from projects under incompatible licenses either; describe behavior and write our own. The repo is public, and this keeps it safe from takedowns.

REA gotchas: it needs Node 22, 24 or 26+; Ghidra can't import universal Mach-O binaries, so thin them first (`lipo -thin arm64 <binary> -output .re/bin/<name>.arm64`).

## Real data

- A user's rekordbox library (`~/Library/Pioneer/rekordbox/master.db` on macOS) and USB sticks are read-only. Copy them into `.re/fixtures/` before experimenting. Never write to a mounted USB stick except as part of an export the user asked for.
- Files that look like stored credentials (e.g. `PIONEER/extracted/gcred.dat` on sticks) are never read or copied.

## Working with agents

Big features are built by several agents in parallel, coordinated by one orchestrator:

1. The orchestrator writes the contract in `docs/specs/<feature>.md` (commands, data formats, events, file ownership) before any agent starts.
2. Each coding agent works in its own git worktree on its own branch, owns specific crates/files, and commits there with tests passing.
3. Research agents write `research/<topic>.md` only.
4. The orchestrator reviews, merges, resolves conflicts (usually the `registry![…]` list and workspace `Cargo.toml`), runs the full checks and updates `docs/status.md`.
