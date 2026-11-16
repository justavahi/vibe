use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;

use dialoguer::{Select, theme::ColorfulTheme};
use openrouter::OpenRouter;
use openrouter::completions::Request;
use openrouter::completions::request::{Content, Message};

use crate::config::Config;

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

/// A command handler. It receives the arguments that follow the command name
/// and returns a boxed future so that sync and async handlers share one type.
pub type Handler =
    for<'a> fn(&'a [String]) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

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
    chat              Start an interactive chat with the selected model

EXAMPLES:
    vibe api-key "sk-or-v1-..."
    vibe model
    vibe chat

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

    let mut models = client.models(&[]).await.unwrap_or_else(|e| {
        exit_with(format!("failed to fetch models: {e}"))
    });

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

/// Handles `vibe chat`: a simple interactive REPL.
pub async fn chat(_args: &[String]) {
    let config = Config::load().unwrap_or_else(|e| exit_with(e));
    let api_key = config.require_api_key().unwrap_or_else(|e| exit_with(e));
    let model = config.require_model().unwrap_or_else(|e| exit_with(e));

    let client = OpenRouter::new(api_key);

    println!("Chatting with {model}. Type your message, or 'exit' / Ctrl-D to quit.\n");

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
            messages: Some(vec![Message::User {
                content: Content::Plain(prompt.to_string()),
                name: None,
                cache_control: None,
            }]),
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
            }
            Err(e) => {
                eprintln!("error: request failed: {}\n", describe_error(&e));
            }
        }
    }

    println!("bye!");
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

/// Prints an error and exits with a non-zero status.
fn exit_with(error: impl std::fmt::Display) -> ! {
    eprintln!("error: {error}");
    std::process::exit(1);
}

/// Builds the command registry: maps a command name to its handler.
///
/// Each handler takes the arguments following the command name and returns a
/// boxed future, so synchronous and asynchronous handlers share a single type.
pub fn registry() -> std::collections::HashMap<&'static str, Handler> {
    fn boxed<'a, F>(fut: F) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>
    where
        F: Future<Output = ()> + Send + 'a,
    {
        Box::pin(fut)
    }

    fn wrap_help(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(help(args))
    }
    fn wrap_api_key(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(set_api_key(args))
    }
    fn wrap_model(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(pick_model(args))
    }
    fn wrap_chat(args: &[String]) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        boxed(chat(args))
    }

    std::collections::HashMap::from([
        ("help", wrap_help as Handler),
        ("-h", wrap_help as Handler),
        ("--help", wrap_help as Handler),
        ("api-key", wrap_api_key as Handler),
        ("model", wrap_model as Handler),
        ("chat", wrap_chat as Handler),
    ])
}
