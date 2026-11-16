use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;

use dialoguer::{Confirm, Select, theme::ColorfulTheme};

use crate::config::{Config, Vendor};
use crate::provider::Provider;
use crate::session::{DEFAULT_SESSION, HistoryEntry, Store};

/// A command handler. It receives the arguments that follow the command name
/// and returns a boxed future so that sync and async handlers share one type.
pub type Handler = for<'a> fn(&'a [String]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Boxes a future into the shared handler return type.
fn boxed<'a, F>(fut: F) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
where
    F: Future<Output = ()> + Send + 'a,
{
    Box::pin(fut)
}

/// Prints the help menu.
pub async fn help(_args: &[String]) {
    println!(
        r#"vibe - a tiny CLI harness for LLM providers

USAGE:
    vibe <COMMAND> [ARGS]

COMMANDS:
    help                    Show this help menu
    vendor [<VENDOR>]       Select the provider (openrouter or venice; default openrouter)
    api-key "<KEY>"         Save a key for the selected provider
    api-key -o "<KEY>"      Save an OpenRouter API key
    api-key -v "<KEY>"      Save a Venice API key
    model                   Pick a model from the selected provider
    code                    Chat with the selected model, keeping session history
    code <name>             Continue a specific session (also: code session <name>)

    vibe session - manage sessions for the current directory
      vibe session add <name>             - create a new session (random name if omitted)
      vibe session new <name>             - create a new session and switch to it
      vibe session list                   - list sessions for the current directory
      vibe session switch, sw <name>      - switch to a session
      vibe session rename, rn <old> <new> - rename a session
      vibe session remove, rm <name>...   - remove locally, move global copies to trash
      vibe session delete, del <name>...  - delete completely (local and global)
      vibe session clean                  - empty this directory's trash

EXAMPLES:
    vibe vendor venice
    vibe api-key -o "sk-or-v1-..."
    vibe model
    vibe code
    vibe session list

Configuration is stored in ~/.config/vibe/config.toml"#
    );
}

/// Handles `vibe api-key [-o|--openrouter|-v|--venice] "<key>"`.
pub async fn set_api_key(args: &[String]) {
    let mut vendor_flag = None;
    let mut key = None;

    for arg in args {
        match arg.as_str() {
            "-o" | "--openrouter" => vendor_flag = Some(Vendor::OpenRouter),
            "-v" | "--venice" => vendor_flag = Some(Vendor::Venice),
            other => key = Some(other.to_string()),
        }
    }

    let Some(key) = key else {
        exit_with("usage: vibe api-key [-o|--openrouter|-v|--venice] \"<key>\"");
    };

    if key == "choose" {
        exit_with("`vibe api-key choose` was removed. Use `vibe vendor <vendor>` instead.");
    }

    if key.trim().is_empty() {
        exit_with("API key cannot be empty");
    }

    let mut config = Config::load().unwrap_or_else(|e| exit_with(e));

    let vendor = match vendor_flag {
        Some(vendor) => vendor,
        None => {
            // No flag: use the stored vendor, defaulting to (and persisting)
            // openrouter when the field is missing or invalid.
            let vendor = config.vendor();
            if !config.has_valid_vendor() {
                config.set_vendor(Vendor::OpenRouter);
            }
            vendor
        }
    };

    config.set_api_key_for(vendor, &key);
    config.save().unwrap_or_else(|e| exit_with(e));

    println!("{vendor} API key saved to {}", Config::path().display());
}

/// Handles `vibe vendor [<vendor>]`: select the active provider.
///
/// With no argument the vendor defaults to `openrouter`. Afterwards, if the
/// configured model is not offered by the new vendor, the user is offered a
/// chance to pick a new one.
pub async fn set_vendor(args: &[String]) {
    let name = args.first().map(String::as_str).unwrap_or("openrouter");

    let Some(vendor) = Vendor::parse(name) else {
        let options = Vendor::ALL
            .iter()
            .map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(" or ");
        exit_with(format!("unknown vendor `{name}`. Use `{options}`."));
    };

    let mut config = Config::load().unwrap_or_else(|e| exit_with(e));
    config.set_vendor(vendor);
    config.save().unwrap_or_else(|e| exit_with(e));

    println!("Provider set to {vendor}");

    // Verify the current model is available from the new vendor.
    let Some(model) = config.llm.model.clone() else {
        return;
    };

    let Some(api_key) = config.api_key_for(vendor) else {
        println!("note: no {vendor} API key set; skipping model check");
        return;
    };

    let provider = Provider::new(vendor, api_key);
    let Ok(models) = provider.models().await else {
        // Can't reach the provider; skip the check silently.
        return;
    };

    if models.iter().any(|m| m.id == model) {
        return;
    }

    let switch = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!(
            "Current model `{model}` does not exist in {vendor}. Would you like to switch your model?"
        ))
        .default(false)
        .interact()
        .unwrap_or_else(|e| exit_with(format!("selection failed: {e}")));

    if switch {
        pick_model(&[]).await;
    }
}

/// Handles `vibe model`: fetches models from the selected provider and lets
/// the user pick one.
pub async fn pick_model(_args: &[String]) {
    let config = Config::load().unwrap_or_else(|e| exit_with(e));
    let vendor = config.vendor();
    let api_key = config.require_api_key().unwrap_or_else(|e| exit_with(e));

    println!("Fetching models from {vendor}...");
    let provider = Provider::new(vendor, api_key);

    let models = provider
        .models()
        .await
        .unwrap_or_else(|e| exit_with(format!("failed to fetch models: {e}")));

    if models.is_empty() {
        exit_with(format!("{vendor} returned no models"));
    }

    let labels: Vec<String> = models
        .iter()
        .map(|m| {
            if m.name.is_empty() || m.name == m.id {
                m.id.clone()
            } else {
                format!("{}  -  {}", m.id, m.name)
            }
        })
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose a model")
        .items(&labels)
        .default(0)
        .interact()
        .unwrap_or_else(|e| exit_with(format!("selection failed: {e}")));

    let chosen = models[selection].id.clone();

    let mut config = Config::load().unwrap_or_else(|e| exit_with(e));
    config.llm.model = Some(chosen.clone());
    config.save().unwrap_or_else(|e| exit_with(e));

    println!("Model set to {chosen}");
}

/// Handles `vibe code`: an interactive REPL backed by a session's history.
pub async fn code(args: &[String]) {
    let config = Config::load().unwrap_or_else(|e| exit_with(e));
    let api_key = config.require_api_key().unwrap_or_else(|e| exit_with(e));
    let model = config.require_model().unwrap_or_else(|e| exit_with(e));

    let requested = match requested_session(args) {
        Ok(requested) => requested,
        Err(msg) => exit_with(msg),
    };

    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.ensure().unwrap_or_else(|e| exit_with(e));

    // Rule 1: with no sessions yet, work in the default session named `session`.
    // Rule 4: once there is a real choice (2+ sessions), let the user pick one.
    // An explicit name (`vibe code <name>`) skips the picker.
    let sessions = store.list().unwrap_or_else(|e| exit_with(e));
    let session = match requested {
        Some(name) => {
            if !store.exists(&name) {
                exit_with(format!(
                    "session `{name}` not found. Create it with `vibe session add {name}`"
                ));
            }
            name
        }
        None => match store.current().filter(|c| store.exists(c)) {
            // Respect a session chosen with `vibe session switch` (or the last one used).
            Some(current) => current,
            None => match sessions.as_slice() {
                [] => {
                    store
                        .ensure_session(DEFAULT_SESSION, Some(model))
                        .unwrap_or_else(|e| exit_with(e));
                    DEFAULT_SESSION.to_string()
                }
                [only] => only.clone(),
                _ => pick_session(&sessions),
            },
        },
    };

    store.set_current(&session).unwrap_or_else(|e| exit_with(e));

    let mut history = store.load_history(&session).unwrap_or_else(|e| exit_with(e));

    let vendor = config.vendor();
    let provider = Provider::new(vendor, api_key);
    println!(
        "Session `{session}` in {}. Chatting with {model} via {vendor}. Type 'exit' / Ctrl-D to quit.\n",
        store.dir_name
    );

    // Replay the existing history so the user sees the context.
    for entry in &history {
        match entry.role.as_str() {
            "user" => println!("you> {}", entry.content),
            "assistant" => println!("vibe> {}\n", entry.content),
            _ => {}
        }
    }

    let stdin = io::stdin();
    loop {
        print!("you> ");
        io::stdout().flush().ok();

        let mut input = String::new();
        match stdin.read_line(&mut input) {
            Ok(0) => break, // EOF (Ctrl-D)
            Ok(_) => {}
            Err(e) => exit_with(format!("failed to read input: {e}")),
        }

        let prompt = input.trim();
        if prompt.is_empty() {
            continue;
        }
        if prompt.eq_ignore_ascii_case("exit") || prompt.eq_ignore_ascii_case("quit") {
            break;
        }

        // Append the user's turn, then send the whole conversation.
        history.push(HistoryEntry {
            role: "user".to_string(),
            content: prompt.to_string(),
        });

        match provider.chat(model, &history).await {
            Ok(answer) => {
                println!("vibe> {answer}\n");

                history.push(HistoryEntry {
                    role: "assistant".to_string(),
                    content: answer,
                });

                store
                    .save_history(&session, &history)
                    .unwrap_or_else(|e| exit_with(e));
            }
            Err(e) => {
                eprintln!("error: request failed: {e}\n");
                // Drop the unanswered turn so history stays consistent.
                history.pop();
            }
        }
    }

    println!("bye!");
}

/// Parses the optional session selector from `vibe code [session] <name>`.
///
/// Accepts both `vibe code <name>` and `vibe code session <name>`.
fn requested_session(args: &[String]) -> Result<Option<String>, String> {
    match args {
        [] => Ok(None),
        [name] => Ok(Some(name.clone())),
        [keyword, name] if keyword == "session" => Ok(Some(name.clone())),
        _ => Err("usage: vibe code [session] <name>".to_string()),
    }
}

/// Shows the session picker (same UI as `vibe model`).
fn pick_session(sessions: &[String]) -> String {
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose a session")
        .items(sessions)
        .default(0)
        .interact()
        .unwrap_or_else(|e| exit_with(format!("selection failed: {e}")));

    sessions[selection].clone()
}

// --- session subcommands ---------------------------------------------------

/// Handles `vibe session <SUBCOMMAND> [ARGS]` using its own handler map.
pub async fn session(args: &[String]) {
    let subcommands = session_registry();

    let Some((sub, rest)) = args.split_first() else {
        session_help();
        return;
    };

    match subcommands.get(sub.as_str()) {
        Some(handler) => handler(rest).await,
        None => {
            eprintln!("error: unknown session subcommand `{sub}`\n");
            session_help();
            std::process::exit(1);
        }
    }
}

/// Prints the session subcommand help.
fn session_help() {
    println!(
        r#"vibe session - manage sessions for the current directory

USAGE:
    vibe session <SUBCOMMAND> [ARGS]

SUBCOMMANDS:
    add <name>                  Create a new session (random name if omitted)
    new <name>                  Create a new session and switch to it
    list                        List sessions for the current directory
    switch, sw <name>           Switch to a session
    rename, rn <old> <new>      Rename a session
    remove, rm <name>...        Remove locally, move global copies to trash
    delete, del <name>...       Delete completely (local and global)
    clean                       Empty this directory's trash

Sessions are stored in ./.vibe/sessions and mirrored to
~/.vibe/dirs/<dir>/sessions."#
    );
}

/// Creates a session from `[name]` (random if omitted) and returns its name.
fn create_session(args: &[String]) -> String {
    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.ensure().unwrap_or_else(|e| exit_with(e));

    let name = match args.first() {
        Some(name) => name.clone(),
        None => crate::session::random_nickname(),
    };

    let model = Config::load().ok().and_then(|c| c.llm.model);
    store
        .create(&name, model.as_deref())
        .unwrap_or_else(|e| exit_with(e));

    name
}

/// `vibe session add [name]`.
async fn session_add(args: &[String]) {
    let name = create_session(args);
    println!("Session `{name}` created");
}

/// `vibe session new [name]`: like `add`, then switches to the new session.
async fn session_new(args: &[String]) {
    let name = create_session(args);

    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.set_current(&name).unwrap_or_else(|e| exit_with(e));

    println!("Session `{name}` created and switched to");
}

/// `vibe session list`.
async fn session_list(_args: &[String]) {
    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    let sessions = store.list().unwrap_or_else(|e| exit_with(e));

    if sessions.is_empty() {
        println!("No sessions in {}", store.dir_name);
        return;
    }

    println!("Sessions in {}:", store.dir_name);
    let current = store.current();
    for name in sessions {
        let meta = store.read_meta(&name).unwrap_or_default();
        let marker = if current.as_deref() == Some(name.as_str()) {
            " (current)"
        } else {
            ""
        };
        println!("  {name}  ({}){marker}", meta.nickname);
    }
}

/// `vibe session rename <old> <new>`.
async fn session_rename(args: &[String]) {
    let (Some(old), Some(new)) = (args.first(), args.get(1)) else {
        exit_with("usage: vibe session rename <old> <new>");
    };

    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.rename(old, new).unwrap_or_else(|e| exit_with(e));

    println!("Session `{old}` renamed to `{new}`");
}

/// `vibe session remove <name>...`.
async fn session_remove(args: &[String]) {
    if args.is_empty() {
        exit_with("usage: vibe session remove <name>...");
    }

    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    let mut failed = false;

    for name in args {
        match store.remove(name) {
            Ok(()) => println!("Session `{name}` removed (global copy moved to trash)"),
            Err(e) => {
                eprintln!("error: {e}");
                failed = true;
            }
        }
    }

    if failed {
        std::process::exit(1);
    }
}

/// `vibe session delete <name>...`.
async fn session_delete(args: &[String]) {
    if args.is_empty() {
        exit_with("usage: vibe session delete <name>...");
    }

    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    let mut failed = false;

    for name in args {
        match store.delete(name) {
            Ok(()) => println!("Session `{name}` deleted"),
            Err(e) => {
                eprintln!("error: {e}");
                failed = true;
            }
        }
    }

    if failed {
        std::process::exit(1);
    }
}

/// `vibe session clean`.
async fn session_clean(_args: &[String]) {
    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.clean().unwrap_or_else(|e| exit_with(e));

    println!("Trash cleaned for {}", store.dir_name);
}

/// `vibe session switch <name>`.
async fn session_switch(args: &[String]) {
    let Some(name) = args.first() else {
        exit_with("usage: vibe session switch <name>");
    };

    let store = Store::new().unwrap_or_else(|e| exit_with(e));

    if !store.exists(name) {
        exit_with(format!(
            "session `{name}` not found. Create it with `vibe session add {name}`"
        ));
    }

    store.set_current(name).unwrap_or_else(|e| exit_with(e));
    println!("Switched to session `{name}`");
}

/// Prints an error and exits with a non-zero status.
fn exit_with(error: impl std::fmt::Display) -> ! {
    eprintln!("error: {error}");
    std::process::exit(1);
}

/// Builds the top-level command registry: maps a command name to its handler.
pub fn registry() -> HashMap<&'static str, Handler> {
    fn wrap_help(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(help(args))
    }
    fn wrap_api_key(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(set_api_key(args))
    }
    fn wrap_vendor(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(set_vendor(args))
    }
    fn wrap_model(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(pick_model(args))
    }
    fn wrap_code(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(code(args))
    }
    fn wrap_session(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session(args))
    }

    HashMap::from([
        ("help", wrap_help as Handler),
        ("-h", wrap_help as Handler),
        ("--help", wrap_help as Handler),
        ("api-key", wrap_api_key as Handler),
        ("vendor", wrap_vendor as Handler),
        ("model", wrap_model as Handler),
        ("code", wrap_code as Handler),
        ("session", wrap_session as Handler),
    ])
}

/// Builds the session subcommand registry, using the same pattern as `registry`.
pub fn session_registry() -> HashMap<&'static str, Handler> {
    fn wrap_add(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_add(args))
    }
    fn wrap_new(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_new(args))
    }
    fn wrap_list(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_list(args))
    }
    fn wrap_rename(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_rename(args))
    }
    fn wrap_remove(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_remove(args))
    }
    fn wrap_delete(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_delete(args))
    }
    fn wrap_clean(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_clean(args))
    }
    fn wrap_switch(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_switch(args))
    }
    fn wrap_help(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_help_cmd(args))
    }

    HashMap::from([
        ("add", wrap_add as Handler),
        ("new", wrap_new as Handler),
        ("list", wrap_list as Handler),
        ("rename", wrap_rename as Handler),
        ("rn", wrap_rename as Handler),
        ("remove", wrap_remove as Handler),
        ("rm", wrap_remove as Handler),
        ("delete", wrap_delete as Handler),
        ("del", wrap_delete as Handler),
        ("clean", wrap_clean as Handler),
        ("switch", wrap_switch as Handler),
        ("sw", wrap_switch as Handler),
        ("help", wrap_help as Handler),
    ])
}

/// `vibe session help`.
async fn session_help_cmd(_args: &[String]) {
    session_help();
}
