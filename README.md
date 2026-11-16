# vibe

A tiny, local-first CLI harness for chatting with large language models through
the [OpenRouter](https://openrouter.ai) and [Venice AI](https://venice.ai) APIs.
Pick a provider, save an API key, choose a model, and chat — configuration lives
in `~/.config/vibe/config.toml`.

## Installation

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (edition 2024, so Rust 1.85+)
- An API key for at least one provider (see [API](#api))

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

`vibe` talks to two OpenAI-compatible providers. You pick one with
`vibe vendor`; `vibe model` and `vibe code` both use whichever is selected.

### OpenRouter

[OpenRouter](https://openrouter.ai) is a unified gateway that routes requests to
many model providers (OpenAI, Anthropic, Google, Meta, and others) behind a
single API.

- **Base URL:** `https://openrouter.ai/api/v1`
- **Auth:** bearer token in the `Authorization` header
- **Endpoints used:** `GET /models`, `POST /chat/completions`

### Venice AI

[Venice AI](https://venice.ai) is a privacy-first, OpenAI-compatible platform.

- **Base URL:** `https://api.venice.ai/api/v1`
- **Auth:** bearer token in the `Authorization` header
- **Endpoints used:** `GET /models`, `POST /chat/completions`

### Getting an API key

**OpenRouter**

1. Sign in at <https://openrouter.ai>.
2. Create a key at <https://openrouter.ai/keys>.
3. Save it with:

   ```sh
   vibe api-key -o "sk-or-v1-..."
   ```

Some models are free; others are billed per token against your OpenRouter
credits. The model list shown by `vibe model` includes each model's pricing.

> **Tip:** avoid batch-only variants (model IDs ending in `:batch`). They cannot
> be used with the chat endpoint and will return an error.

**Venice AI**

1. Sign in at <https://venice.ai>.
2. Create a key at <https://venice.ai/settings/api/keys>.
3. Save it with:

   ```sh
   vibe api-key -v "..."
   ```

Venice usage is billed against your Venice credits.

### Selecting the provider

```sh
vibe vendor             # select openrouter (the default)
vibe vendor venice      # select venice
vibe vendor openrouter  # select openrouter explicitly
```

The choice is stored as `vendor` in the `[API]` section. It decides which
provider `vibe model` lists models from and which provider `vibe code` chats
with. With no argument, the vendor defaults to `openrouter`; an unknown name is
rejected.

After switching, vibe checks whether the model saved in `[LLM]` still exists on
the new provider. If it does not, you are asked whether to pick a new one:

```
? Current model `deepseek/deepseek-v4.1-flash` does not exist in venice.
  Would you like to switch your model? (y/n)
```

Answering `yes` runs `vibe model` for the new provider. The check is skipped if
no model is saved, if the new provider has no API key yet, or if the provider
cannot be reached.

### Configuration

Settings are stored in `~/.config/vibe/config.toml`, split into an `[API]`
section and an `[LLM]` section:

```toml
[API]
vendor = "openrouter"
openrouter_api_key = "sk-or-v1-..."
venice_api_key = "..."

[LLM]
model = "openai/gpt-4o-mini"
```

The directory is created automatically on first run. If `XDG_CONFIG_HOME` is
set, the file lives at `$XDG_CONFIG_HOME/vibe/config.toml` instead. The file
holds secrets, so keep it private. Every field is written by the commands
below — you normally don't edit it by hand.

- `vendor` — the selected provider. If it is missing or invalid, vibe defaults
  to `openrouter` (and writes that back when you run `vibe api-key`).
- `openrouter_api_key` / `venice_api_key` — one key per provider.
- `model` — the model id used by `vibe code`. Switching providers does not
  change it; `vibe vendor` offers to re-pick it when the model is not available
  on the new provider (see [Selecting the provider](#selecting-the-provider)).

Older flat configs (top-level `api_key` / `model`) are migrated into the new
sections automatically the first time they are loaded.

## Commands

```
vibe <COMMAND> [ARGS]
```

| Command                  | Description                                                     |
| ------------------------ | --------------------------------------------------------------- |
| `vibe help`              | Show the help menu (`-h` and `--help` also work)                |
| `vibe vendor [<VENDOR>]` | Select the provider (`openrouter` or `venice`; default `openrouter`) |
| `vibe api-key "<KEY>"`   | Save a key for the selected provider                            |
| `vibe api-key -o "<KEY>"`| Save an OpenRouter API key                                      |
| `vibe api-key -v "<KEY>"`| Save a Venice API key                                           |
| `vibe model`             | Fetch models from the selected provider and pick one            |
| `vibe code`              | Chat with the selected model, keeping session history           |
| `vibe code <name>`       | Continue a specific session (also `vibe code session <name>`)   |
| `vibe session <SUB>`     | Manage sessions (see [Sessions](#sessions))                     |

### Examples

```sh
# Choose a provider, then save its key
vibe vendor venice
vibe api-key -v "..."             # or: vibe api-key -o "sk-or-v1-..."

# Choose a model from a live list
vibe model

# Start chatting (creates/continues a session for this directory)
vibe code
```

Inside `vibe code`, type a message and press Enter. Type `exit` (or `quit`), or
press `Ctrl-D`, to leave.

### Typical first run

```sh
vibe vendor                        # select openrouter (the default)
vibe api-key -o "sk-or-v1-..."    # save an OpenRouter key
vibe model                         # pick a model
vibe code                          # chat
```

## Sessions

Every directory keeps its own conversations. On the first `vibe code` run in a
directory, vibe creates a default session named `session` and uses it
automatically. After that, `vibe code` resumes the **current** session for the
directory.

The current session is the one most recently used with `vibe code`, or the one
you picked with `vibe session switch <name>`. When no current session is set and
the directory has **two or more** sessions, `vibe code` shows a picker (the same
UI as `vibe model`) so you can choose which one to continue.

To go straight to a specific session, either switch to it or pass its name:

```sh
vibe session switch notes   # make "notes" the current session
vibe code                   # resumes "notes"

vibe code notes             # continue "notes" for this run
vibe code session notes     # same thing, explicit form
```

To start a fresh conversation, create a new session and switch to it in one step:

```sh
vibe session new scratch    # create "scratch" and switch to it
vibe code                   # starts fresh in "scratch"
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
  current            # name of the session most recently used here
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
| `new <name>`                  | Like `add`, and also switch to the new session                  |
| `list`                        | List sessions for the current directory                         |
| `switch, sw <name>`           | Make `<name>` the current session for this directory            |
| `rename, rn <old> <new>`      | Rename a session (local and global)                             |
| `remove, rm <name>...`        | Remove locally; move global copies to `~/.vibe/trash`           |
| `delete, del <name>...`       | Delete completely, from both `.vibe` and `~/.vibe`              |
| `clean`                       | Empty this directory's trash                                    |
| `help`                        | Show session subcommand help                                    |

`list` prints each session's name together with its random nickname. The session
most recently used with `vibe code` in this directory is marked `(current)`:

```
Sessions in ~-a-b:
  notes  (k5o0328s)
  session  (zla85706) (current)
```

The current session is recorded in `./.vibe/current` and is updated on every
`vibe code` run. Renaming or removing that session updates or clears the marker
automatically.

Session names must be unique within a directory and cannot contain `/`, `\`, or
be `.` / `..`.

`remove` and `delete` accept one or more names. When given several, they process
each one and report any that were not found, rather than stopping at the first
error.

```sh
vibe session add notes          # create a session named "notes"
vibe session new notes          # create "notes" and switch to it
vibe session add                # create one with a random name
vibe session list               # see sessions for this directory
vibe session switch notes       # make "notes" the current session
vibe session rn notes ideas     # rename "notes" to "ideas"
vibe session rm ideas           # remove locally, keep a copy in trash
vibe session rm a b c           # remove several at once
vibe session clean              # empty the trash for this directory
vibe session del ideas          # delete everywhere
vibe session del a b c          # delete several at once
```
