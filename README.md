# vibe

A tiny, local-first CLI harness for chatting with large language models through
the [OpenRouter](https://openrouter.ai) API. Pick a model once, save your API
key, and chat — configuration lives in `~/.config/vibe/config.toml`.

## Installation

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (edition 2024, so Rust 1.85+)
- An OpenRouter account and API key (see [API](#api))

### Build from source

```sh
git clone <your-repo-url>
cd vibe
cargo build --release
```

The binary is produced at `target/release/vibe`. Optionally install it onto your
`PATH`:

```sh
cargo install --path .
```

### NixOS / minimal systems

`vibe` uses [rustls](https://github.com/rustls/rustls) rather than system
OpenSSL, so no `openssl`, `pkg-config`, or other system libraries are required.
The `openrouter` crate is vendored under `vendor/openrouter/` with its TLS
backend switched to rustls for this reason.

## API

`vibe` talks to [OpenRouter](https://openrouter.ai), a unified gateway that
routes requests to many model providers (OpenAI, Anthropic, Google, Meta, and
others) behind a single, OpenAI-compatible API.

- **Base URL:** `https://openrouter.ai/api/v1`
- **Auth:** a bearer token in the `Authorization` header — your OpenRouter API key
- **Endpoints used:**
  - `GET /models` — list available models (used by `vibe model`)
  - `POST /chat/completions` — send a chat request (used by `vibe code`)

### Getting an API key

1. Sign in at <https://openrouter.ai>.
2. Create a key at <https://openrouter.ai/keys>.
3. Save it locally with:

   ```sh
   vibe api-key "sk-or-v1-..."
   ```

Some models are free; others are billed per token against your OpenRouter
credits. The model list shown by `vibe model` includes each model's pricing.

> **Tip:** avoid batch-only variants (model IDs ending in `:batch`). They cannot
> be used with the chat endpoint and will return an error.

### Configuration

Settings are stored in `~/.config/vibe/config.toml`:

```toml
api_key = "sk-or-v1-..."
model = "openai/gpt-4o-mini"
```

The directory is created automatically on first run. If `XDG_CONFIG_HOME` is
set, the file lives at `$XDG_CONFIG_HOME/vibe/config.toml` instead. The file
holds a secret, so keep it private. Both keys are written by the commands below —
you normally don't edit it by hand.

## Commands

```
vibe <COMMAND> [ARGS]
```

| Command              | Description                                                    |
| -------------------- | -------------------------------------------------------------- |
| `vibe help`          | Show the help menu (`-h` and `--help` also work)               |
| `vibe api-key "<KEY>"` | Save your OpenRouter API key to the config file              |
| `vibe model`         | Fetch the available models and pick one interactively          |
| `vibe code`          | Chat with the selected model, keeping session history          |
| `vibe code <name>`   | Continue a specific session (also `vibe code session <name>`)  |
| `vibe session <SUB>` | Manage sessions (see [Sessions](#sessions))                    |

### Examples

```sh
# Save your API key
vibe api-key "sk-or-v1-..."

# Choose a model from a live, searchable list
vibe model

# Start chatting (creates/continues a session for this directory)
vibe code
```

Inside `vibe code`, type a message and press Enter. Type `exit` (or `quit`), or
press `Ctrl-D`, to leave.

### Typical first run

```sh
vibe api-key "sk-or-v1-..."
vibe model
vibe code
```

## Sessions

Every directory keeps its own conversations. On the first `vibe code` run in a
directory, vibe creates a default session named `session` and uses it
automatically. Once a directory has **two or more** sessions, `vibe code` shows a
picker (the same UI as `vibe model`) so you can choose which one to continue.

To skip the picker and go straight to a saved session, pass its name:

```sh
vibe code notes            # continue the session named "notes"
vibe code session notes    # same thing, explicit form
```

If the named session does not exist, vibe tells you and suggests
`vibe session add <name>`.

Each session stores its full message history and metadata, which is replayed to
the model as context on every turn. Every session also gets a random 8-character
nickname when it is created; `vibe session list` shows it next to the name.

### Storage layout

Sessions are written in two places at once.

**Locally**, in the directory you run vibe from:

```
.vibe/
  sessions/
    session/
      files/
        history.json
        meta.json
    notes/
      files/
        history.json
        meta.json
```

**Globally**, mirrored under your home directory:

```
~/.vibe/
  dirs/
    <dir-name>/
      sessions/
        session/
          files/
            history.json
            meta.json
  trash/
    <dir-name>/
      sessions/        # sessions removed with `vibe session remove`
```

`<dir-name>` is the directory's full path with `$HOME` replaced by `~` and every
`/` replaced by `-`. For example `/home/you/a/b` becomes `~-a-b`.

`meta.json` holds the session's random 8-character nickname, creation and
last-used timestamps, and the model that was active when it was created.

Both `.vibe/` (in the working directory) and `~/.vibe/` are created
automatically when needed. `.vibe/` is listed in `.gitignore` so local session
data is never committed.

### Managing sessions

```
vibe session <SUBCOMMAND> [ARGS]
```

| Subcommand                    | Description                                                     |
| ----------------------------- | --------------------------------------------------------------- |
| `add <name>`                  | Create a new session (a random 8-char name is used if omitted)  |
| `list`                        | List sessions for the current directory                         |
| `rename, rn <old> <new>`      | Rename a session (local and global)                             |
| `remove, rm <name>`           | Remove locally; move the global copy to `~/.vibe/trash`         |
| `delete, del <name>`          | Delete completely, from both `.vibe` and `~/.vibe`              |
| `clean`                       | Empty this directory's trash                                    |
| `help`                        | Show session subcommand help                                    |

`list` prints each session's name together with its random nickname:

```
Sessions in ~-a-b:
  notes  (k5o0328s)
  session  (zla85706)
```

Session names must be unique within a directory and cannot contain `/`, `\`, or
be `.` / `..`.

```sh
vibe session add notes          # create a session named "notes"
vibe session add                # create one with a random name
vibe session list               # see sessions for this directory
vibe session rn notes ideas     # rename "notes" to "ideas"
vibe session rm ideas           # remove locally, keep a copy in trash
vibe session clean              # empty the trash for this directory
vibe session del ideas          # delete everywhere
```
