use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};
use tuneox_audio::{Engine, EngineEvent};
use tuneox_commands::{Context, Effect};
use tuneox_library::Library;

type Shared = Arc<Mutex<Context>>;

/// The command contexts, or why the library couldn't be opened.
struct Core(Result<Contexts, String>);

#[derive(Clone)]
struct Contexts {
    /// Library work: imports, searches, edits.
    library: Shared,
    /// Deck and mixer commands, on their own library connection and sharing
    /// the audio engine, so a long import never holds up a deck.
    player: Shared,
    /// Names of the commands that go to `player`.
    playback: Arc<HashSet<&'static str>>,
}

/// Runs any TuneOx command by name. The UI is a client of the same command
/// registry as the CLI and MCP server; progress arrives as
/// `tuneox://progress` events, deck and mixer state as `tuneox://deck` and
/// `tuneox://mixer`.
#[tauri::command]
async fn run_command(
    app: AppHandle,
    core: State<'_, Core>,
    name: String,
    input: Value,
) -> Result<Value, String> {
    let contexts = core.0.clone()?;
    let ctx = if contexts.playback.contains(name.as_str()) {
        contexts.player
    } else {
        contexts.library
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut ctx = ctx
            .lock()
            .map_err(|_| "the library is unavailable after an earlier crash".to_string())?;
        let events = app.clone();
        let command = name.clone();
        ctx.set_progress(move |p| {
            let _ = events.emit(
                "tuneox://progress",
                json!({ "command": command, "stage": p.stage, "done": p.done, "total": p.total }),
            );
        });
        let result = tuneox_commands::run_json(&mut ctx, &name, input).map_err(|e| e.to_string());
        ctx.set_progress(|_| {});
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

fn open_contexts() -> Result<Contexts, String> {
    let path = Library::default_path().ok_or("no default library location on this OS")?;
    let open = || Library::open(&path).map_err(|e| e.to_string());
    let mut player = Context::new(open()?, Some(path.clone()));
    let mut library = Context::new(open()?, Some(path.clone()));
    // Started at launch so the window gets deck events from the start. It
    // returns at once; the device opens in the background.
    match player.engine() {
        Ok(engine) => library = library.with_engine(engine.clone()),
        Err(e) => eprintln!("audio engine unavailable: {e}"),
    }
    let playback = tuneox_commands::catalog()
        .into_iter()
        .filter(|c| c.effect == Effect::Playback)
        .map(|c| c.name)
        .collect();
    Ok(Contexts {
        library: Arc::new(Mutex::new(library)),
        player: Arc::new(Mutex::new(player)),
        playback: Arc::new(playback),
    })
}

/// Forwards engine events to the window.
fn forward_events(app: &AppHandle, engine: &Engine) {
    let app = app.clone();
    engine.on_event(move |event| {
        let _ = match event {
            EngineEvent::Deck(deck) => app.emit("tuneox://deck", deck),
            EngineEvent::Mixer(mixer) => app.emit("tuneox://mixer", mixer),
        };
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let contexts = open_contexts();
    let engine = contexts.as_ref().ok().and_then(|c| {
        c.player
            .lock()
            .ok()
            .and_then(|ctx| ctx.running_engine().cloned())
    });

    tauri::Builder::default()
        .manage(Core(contexts))
        .setup(move |app| {
            if let Some(engine) = &engine {
                forward_events(app.handle(), engine);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![run_command])
        .run(tauri::generate_context!())
        .expect("error while running TuneOx");
}
