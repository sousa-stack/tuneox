//! `tuneox`: the TuneOx command-line tool.
//!
//! Every subcommand is a thin wrapper over a command in `tuneox-commands`,
//! the same registry the desktop app and MCP server use. `--json` prints the
//! command's raw output; `tuneox run` calls any command by name.

use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use tuneox_commands::{Context, Progress};
use tuneox_library::Library;

#[derive(Parser)]
#[command(
    name = "tuneox",
    version,
    about = "TuneOx, the open-source rekordbox alternative"
)]
struct Cli {
    /// Library database to use. Defaults to $TUNEOX_LIBRARY, then the
    /// standard location for this OS.
    #[arg(long, global = true)]
    library: Option<PathBuf>,
    /// Print raw JSON output.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Import tracks, playlists, cues and beat grids from rekordbox.
    ImportRekordbox {
        /// rekordbox's data folder (holds master.db and share/).
        #[arg(long)]
        rekordbox_dir: Option<String>,
        /// Import from this master.db instead, e.g. a backup.
        #[arg(long)]
        master_db: Option<String>,
        /// Show what would be imported without changing the library.
        #[arg(long)]
        dry_run: bool,
    },
    /// Counts of what's in the library.
    Summary,
    /// The playlist tree.
    Playlists,
    /// Find tracks.
    Search {
        /// Words to match (title, artist, album, genre, label, remixer, comment).
        text: Vec<String>,
        #[arg(long)]
        bpm_min: Option<f64>,
        #[arg(long)]
        bpm_max: Option<f64>,
        /// Exact key, e.g. Am, F#m, 8A.
        #[arg(long)]
        key: Option<String>,
        /// Keys that mix harmonically with this one.
        #[arg(long)]
        compatible_with: Option<String>,
        #[arg(long)]
        genre: Option<String>,
        #[arg(long)]
        playlist: Option<i64>,
        #[arg(long, default_value_t = 50)]
        limit: u32,
    },
    /// Details, cues and beat grid of one track.
    Track { id: i64 },
    /// List every command with its JSON schemas (the MCP tool catalog).
    Commands,
    /// Run the MCP server on stdin/stdout, for Claude and other MCP clients.
    Mcp,
    /// Run any command by name with JSON input.
    Run {
        name: String,
        /// JSON input; defaults to {}.
        input: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    if let Cmd::Commands = cli.command {
        let catalog = tuneox_commands::catalog();
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&catalog)?);
        } else {
            for c in catalog {
                let effect = serde_json::to_value(c.effect)?;
                println!(
                    "{:<18} {:<12} {}",
                    c.name,
                    effect.as_str().unwrap_or(""),
                    c.description
                );
            }
        }
        return Ok(());
    }

    let library_path = cli
        .library
        .or_else(|| std::env::var_os("TUNEOX_LIBRARY").map(PathBuf::from))
        .or_else(|| dirs::data_dir().map(|d| d.join("TuneOx").join("library.db")))
        .ok_or("no default library location; pass --library")?;
    let library = Library::open(&library_path)?;
    let mut ctx = Context::new(library, Some(library_path)).on_progress(show_progress);

    if let Cmd::Mcp = cli.command {
        // stdout carries protocol messages only; diagnostics go to stderr.
        tuneox_mcp::serve(&mut ctx, std::io::stdin().lock(), std::io::stdout())?;
        return Ok(());
    }

    let (name, input) = match cli.command {
        Cmd::ImportRekordbox {
            rekordbox_dir,
            master_db,
            dry_run,
        } => (
            "import_rekordbox",
            json!({ "rekordbox_dir": rekordbox_dir, "master_db": master_db, "dry_run": dry_run }),
        ),
        Cmd::Summary => ("library_summary", json!({})),
        Cmd::Playlists => ("list_playlists", json!({})),
        Cmd::Search {
            text,
            bpm_min,
            bpm_max,
            key,
            compatible_with,
            genre,
            playlist,
            limit,
        } => (
            "search_tracks",
            json!({
                "text": (!text.is_empty()).then(|| text.join(" ")),
                "bpm_min": bpm_min, "bpm_max": bpm_max, "key": key,
                "compatible_with_key": compatible_with, "genre": genre,
                "playlist_id": playlist, "limit": limit,
            }),
        ),
        Cmd::Track { id } => ("get_track", json!({ "id": id })),
        Cmd::Run { name, input } => {
            let input: Value = serde_json::from_str(input.as_deref().unwrap_or("{}"))?;
            let output = tuneox_commands::run_json(&mut ctx, &name, strip_nulls(input))?;
            println!("{}", serde_json::to_string_pretty(&output)?);
            return Ok(());
        }
        Cmd::Commands | Cmd::Mcp => unreachable!(),
    };

    let output = tuneox_commands::run_json(&mut ctx, name, strip_nulls(input))?;
    finish_progress();
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print_human(name, &output);
    }
    Ok(())
}

/// Optional fields are omitted rather than sent as null.
fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            Value::Object(map.into_iter().filter(|(_, v)| !v.is_null()).collect())
        }
        other => other,
    }
}

fn show_progress(p: &Progress) {
    if std::io::stderr().is_terminal() {
        eprint!("\r\x1b[K{}… {}/{}", p.stage, p.done, p.total);
    }
}

fn finish_progress() {
    if std::io::stderr().is_terminal() {
        eprint!("\r\x1b[K");
    }
}

fn print_human(name: &str, out: &Value) {
    match name {
        "import_rekordbox" => print_import(out),
        "library_summary" => {
            for key in ["tracks", "playlists", "folders", "cues", "beat_grids"] {
                println!("{:<11} {}", key.replace('_', " "), out[key]);
            }
            if let Some(path) = out["library_path"].as_str() {
                println!("{:<11} {path}", "library");
            }
        }
        "list_playlists" => print_playlists(out),
        "search_tracks" => {
            let tracks = out["tracks"].as_array().cloned().unwrap_or_default();
            for t in &tracks {
                println!("{}", track_line(t));
            }
            if out["truncated"].as_bool() == Some(true) {
                println!("… more results; raise --limit");
            } else if tracks.is_empty() {
                println!("no tracks found");
            }
        }
        "get_track" => print_track(out),
        _ => println!("{}", serde_json::to_string_pretty(out).unwrap_or_default()),
    }
}

fn print_import(r: &Value) {
    let counts = |v: &Value| {
        format!(
            "{} found, {} new, {} updated",
            v["found"], v["created"], v["updated"]
        )
    };
    if r["dry_run"].as_bool() == Some(true) {
        println!("Dry run: nothing was changed.");
    }
    println!("tracks       {}", counts(&r["tracks"]));
    println!(
        "playlists    {} ({} smart)",
        counts(&r["playlists"]),
        r["smart_playlists"]
    );
    println!("cues         {}", r["cues"]);
    println!("beat grids   {}", r["beat_grids"]);
    for (label, key) in [
        ("missing audio files", "missing_audio_files"),
        ("missing analysis files", "missing_analysis_files"),
        ("unreadable analysis files", "unreadable_analysis_files"),
        ("merged duplicate tracks", "merged_duplicates"),
        ("skipped playlist entries", "skipped_playlist_entries"),
    ] {
        if r[key].as_u64().unwrap_or(0) > 0 {
            println!("{label}: {}", r[key]);
        }
    }
    for w in r["warnings"].as_array().into_iter().flatten() {
        println!("warning: {}", w.as_str().unwrap_or_default());
    }
    println!("took {} ms", r["elapsed_ms"]);
}

fn print_playlists(out: &Value) {
    let playlists = out["playlists"].as_array().cloned().unwrap_or_default();
    let mut depth: HashMap<i64, usize> = HashMap::new();
    for p in &playlists {
        let d = p["parent_id"]
            .as_i64()
            .and_then(|parent| depth.get(&parent))
            .map_or(0, |d| d + 1);
        depth.insert(p["id"].as_i64().unwrap_or(0), d);
        let name = p["name"].as_str().unwrap_or("");
        match p["kind"].as_str() {
            Some("folder") => println!("{}{name}/", "  ".repeat(d)),
            kind => println!(
                "{}{name}  ({} tracks{}) #{}",
                "  ".repeat(d),
                p["track_count"],
                if kind == Some("smart") { ", smart" } else { "" },
                p["id"]
            ),
        }
    }
}

fn track_line(t: &Value) -> String {
    let artist = t["artist"].as_str().unwrap_or("?");
    let title = t["title"].as_str().unwrap_or("");
    let bpm = t["bpm"]
        .as_f64()
        .map(|b| format!("{b:.2}"))
        .unwrap_or_default();
    let key = t["key"].as_str().unwrap_or("");
    let secs = t["duration_secs"].as_f64().unwrap_or(0.0) as u64;
    format!(
        "#{:<7} {artist} – {title}  [{bpm} BPM, {key}, {}:{:02}]",
        t["id"],
        secs / 60,
        secs % 60
    )
}

fn print_track(out: &Value) {
    let t = &out["track"];
    println!("{}", track_line(t));
    for (label, key) in [
        ("album", "album"),
        ("genre", "genre"),
        ("label", "label"),
        ("file", "path"),
    ] {
        if let Some(v) = t[key].as_str() {
            println!("  {label:<6} {v}");
        }
    }
    if let Some(grid) = out["beat_grid"].as_object() {
        let bpm = grid["constant_bpm"]
            .as_f64()
            .map_or("variable".to_string(), |b| format!("{b:.2} BPM"));
        println!(
            "  grid   {} beats, {bpm}, first beat at {:.3} s",
            grid["beats"],
            grid["first_beat_us"].as_i64().unwrap_or(0) as f64 / 1e6
        );
    }
    for cue in out["cues"].as_array().into_iter().flatten() {
        let name = match cue["hot_cue"].as_u64() {
            Some(slot) => format!("hot {}", (b'A' + slot as u8) as char),
            None => "memory".to_string(),
        };
        let start = cue["start_us"].as_i64().unwrap_or(0) as f64 / 1e6;
        let span = match cue["end_us"].as_i64() {
            Some(end) => format!("{start:.3}–{:.3} s (loop)", end as f64 / 1e6),
            None => format!("{start:.3} s"),
        };
        let comment = cue["comment"].as_str().unwrap_or("");
        println!("  {name:<6} {span} {comment}");
    }
}
