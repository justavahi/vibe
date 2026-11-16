mod commands;
mod config;
mod provider;
mod session;

#[tokio::main]
async fn main() {
    // Make sure the config directory (~/.config/vibe) exists before any command runs.
    let config_dir = config::Config::dir();
    if !config_dir.exists() {
        if let Err(e) = config::Config::ensure_dir() {
            eprintln!(
                "error: could not create config directory {}: {e}",
                config_dir.display()
            );
            std::process::exit(1);
        }
    }

    // Command registry: name -> handler. Handlers receive the args after the command.
    let commands = commands::registry();

    let args: Vec<String> = std::env::args().skip(1).collect();

    // No command given: show help.
    let Some((command, rest)) = args.split_first() else {
        if let Some(handler) = commands.get("help") {
            handler(&[]).await;
        }
        return;
    };

    match commands.get(command.as_str()) {
        Some(handler) => handler(rest).await,
        None => {
            eprintln!("error: unknown command `{command}`\n");
            if let Some(handler) = commands.get("help") {
                handler(&[]).await;
            }
            std::process::exit(1);
        }
    }
}
