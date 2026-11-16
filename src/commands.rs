use std::collections::HashMap;
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;

use dialoguer::{Select, theme::ColorfulTheme};
use openrouter::OpenRouter;
use openrouter::completions::Request;
use openrouter::completions::request::{Content, Message};

use crate::config::Config;
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

/// Formats an OpenRouter error with the underlying code/message, which the
/// library's `Display` impl otherwise hides.
fn describe_error(error: &openrouter::Error) -> String {
    match error {
        openrouter::Error::OpenRouter(e) => {
            format!("OpenRouter error {}: {}", e.code, e.message)
        }
        other => other.to_string(),
    }
}

/// Prints the help menu.
pub async fn help(_args: &[String]) {
    println!(
        r#"vibe - a tiny OpenRouter CLI

USAGE:
    vibe <COMMAND> [ARGS]

COMMANDS:
    help              Show this help menu
    api-key "<KEY>"   Save your OpenRouter API key to the config file
    model             Pick a model interactively and save it to the config file
    code              Chat with the selected model, keeping session history
    code <name>       Continue a specific session (also: code session <name>)

    vibe session - manage sessions for the current directory
      vibe session add <name>             - create a new session (random name if omitted)
      vibe session list                   - list sessions for the current directory
      vibe session rename, rn <old> <new> - rename a session
      vibe session remove, rm <name>      - remove locally, move global copy to trash
      vibe session delete, del <name>     - delete completely (local and global)
      vibe session clean                  - empty this directory's trash

EXAMPLES:
    vibe api-key "sk-or-v1-..."
    vibe model
    vibe code
    vibe code session
    vibe code notes
    vibe session list

Configuration is stored in ~/.config/vibe/config.toml"#
    );
}

/// Handles `vibe api-key "<key>"`.
pub async fn set_api_key(args: &[String]) {
    let Some(key) = args.first() else {
        eprintln!("error: `vibe api-key` requires a key, e.g. vibe api-key \"sk-or-v1-...\"");
        std::process::exit(1);
    };

    if key.trim().is_empty() {
        eprintln!("error: API key cannot be empty");
        std::process::exit(1);
    }

    let mut config = Config::load().unwrap_or_else(|e| exit_with(e));
    config.api_key = Some(key.trim().to_string());

    config.save().unwrap_or_else(|e| exit_with(e));
    println!("API key saved to {}", Config::path().display());
}

/// Handles `vibe model`: fetches the available models and lets the user pick one.
pub async fn pick_model(_args: &[String]) {
    let config = Config::load().unwrap_or_else(|e| exit_with(e));
    let api_key = config.require_api_key().unwrap_or_else(|e| exit_with(e));

    println!("Fetching models from OpenRouter...");
    let client = OpenRouter::new(api_key);

    let mut models = client
        .models(&[])
        .await
        .unwrap_or_else(|e| exit_with(format!("failed to fetch models: {e}")));

    if models.is_empty() {
        exit_with("OpenRouter returned no models");
    }

    models.sort_by(|a, b| a.id.cmp(&b.id));
    let labels: Vec<String> = models
        .iter()
        .map(|m| format!("{}  -  {}", m.id, m.name))
        .collect();

    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Choose a model")
        .items(&labels)
        .default(0)
        .interact()
        .unwrap_or_else(|e| exit_with(format!("selection failed: {e}")));

    let chosen = models[selection].id.clone();

    let mut config = Config::load().unwrap_or_else(|e| exit_with(e));
    config.model = Some(chosen.clone());
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
    };

    let mut history = store.load_history(&session).unwrap_or_else(|e| exit_with(e));

    let client = OpenRouter::new(api_key);
    println!(
        "Session `{session}` in {}. Chatting with {model}. Type 'exit' / Ctrl-D to quit.\n",
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

        let request = Request {
            model: Some(model.to_string()),
            messages: Some(build_messages(&history, prompt)),
            ..Request::default()
        };

        match client.chat_completion(request).await {
            Ok(response) => {
                let answer = response
                    .choices
                    .first()
                    .and_then(extract_text)
                    .unwrap_or_else(|| "<empty response>".to_string());

                println!("vibe> {answer}\n");

                history.push(HistoryEntry {
                    role: "user".to_string(),
                    content: prompt.to_string(),
                });
                history.push(HistoryEntry {
                    role: "assistant".to_string(),
                    content: answer,
                });

                store
                    .save_history(&session, &history)
                    .unwrap_or_else(|e| exit_with(e));
            }
            Err(e) => {
                eprintln!("error: request failed: {}\n", describe_error(&e));
            }
        }
    }

    println!("bye!");
}

/// Turns stored history plus a new prompt into the messages to send.
fn build_messages(history: &[HistoryEntry], prompt: &str) -> Vec<Message> {
    let mut messages = Vec::with_capacity(history.len() + 1);

    for entry in history {
        match entry.role.as_str() {
            "user" => messages.push(Message::User {
                content: Content::Plain(entry.content.clone()),
                name: None,
                cache_control: None,
            }),
            "assistant" => messages.push(Message::Assistant {
                content: Some(Content::Plain(entry.content.clone())),
                name: None,
                tool_calls: None,
            }),
            _ => {}
        }
    }

    messages.push(Message::User {
        content: Content::Plain(prompt.to_string()),
        name: None,
        cache_control: None,
    });

    messages
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

/// Pulls the assistant text out of a completion choice.
fn extract_text(choice: &openrouter::completions::response::Choice) -> Option<String> {
    use openrouter::completions::response::Choice;

    match choice {
        Choice::NonStreaming(c) => c.message.content.clone(),
        Choice::NonChat(c) => Some(c.text.clone()),
        Choice::Streaming(c) => c.delta.content.clone(),
    }
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
    list                        List sessions for the current directory
    rename, rn <old> <new>      Rename a session
    remove, rm <name>           Remove locally, move global copy to trash
    delete, del <name>          Delete completely (local and global)
    clean                       Empty this directory's trash

Sessions are stored in ./.vibe/sessions and mirrored to
~/.vibe/dirs/<dir>/sessions."#
    );
}

/// `vibe session add [name]`.
async fn session_add(args: &[String]) {
    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.ensure().unwrap_or_else(|e| exit_with(e));

    let name = match args.first() {
        Some(name) => name.clone(),
        None => crate::session::random_nickname(),
    };

    let model = Config::load().ok().and_then(|c| c.model);
    store
        .create(&name, model.as_deref())
        .unwrap_or_else(|e| exit_with(e));

    println!("Session `{name}` created");
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
    for name in sessions {
        let meta = store.read_meta(&name).unwrap_or_default();
        println!("  {name}  ({})", meta.nickname);
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

/// `vibe session remove <name>`.
async fn session_remove(args: &[String]) {
    let Some(name) = args.first() else {
        exit_with("usage: vibe session remove <name>");
    };

    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.remove(name).unwrap_or_else(|e| exit_with(e));

    println!("Session `{name}` removed (global copy moved to trash)");
}

/// `vibe session delete <name>`.
async fn session_delete(args: &[String]) {
    let Some(name) = args.first() else {
        exit_with("usage: vibe session delete <name>");
    };

    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.delete(name).unwrap_or_else(|e| exit_with(e));

    println!("Session `{name}` deleted");
}

/// `vibe session clean`.
async fn session_clean(_args: &[String]) {
    let store = Store::new().unwrap_or_else(|e| exit_with(e));
    store.clean().unwrap_or_else(|e| exit_with(e));

    println!("Trash cleaned for {}", store.dir_name);
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
    fn wrap_help(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(session_help_cmd(args))
    }

    HashMap::from([
        ("add", wrap_add as Handler),
        ("list", wrap_list as Handler),
        ("rename", wrap_rename as Handler),
        ("rn", wrap_rename as Handler),
        ("remove", wrap_remove as Handler),
        ("rm", wrap_remove as Handler),
        ("delete", wrap_delete as Handler),
        ("del", wrap_delete as Handler),
        ("clean", wrap_clean as Handler),
        ("help", wrap_help as Handler),
    ])
}

/// `vibe session help`.
async fn session_help_cmd(_args: &[String]) {
    session_help();
}
