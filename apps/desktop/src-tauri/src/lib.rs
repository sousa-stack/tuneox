use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, State};
use tuneox_commands::Context;
use tuneox_library::Library;

/// The command context, or why the library couldn't be opened.
struct Core(Result<Arc<Mutex<Context>>, String>);

/// Runs any TuneOx command by name. The UI is a client of the same command
/// registry as the CLI and MCP server; progress arrives as
/// `tuneox://progress` events.
#[tauri::command]
async fn run_command(
    app: AppHandle,
    core: State<'_, Core>,
    name: String,
    input: Value,
) -> Result<Value, String> {
    let ctx = core.0.clone()?;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let core = Library::default_path()
        .ok_or_else(|| "no default library location on this OS".to_string())
        .and_then(|path| {
            Library::open(&path)
                .map(|library| Context::new(library, Some(path)))
                .map_err(|e| e.to_string())
        })
        .map(|ctx| Arc::new(Mutex::new(ctx)));

    tauri::Builder::default()
        .manage(Core(core))
        .invoke_handler(tauri::generate_handler![run_command])
        .run(tauri::generate_context!())
        .expect("error while running TuneOx");
}
