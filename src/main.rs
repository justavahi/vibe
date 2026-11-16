mod commands;
mod config;

#[tokio::main]
async fn main() {
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
