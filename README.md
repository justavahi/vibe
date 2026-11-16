# vibe

A tiny, local-first CLI harness for chatting with large language models through
the [OpenRouter](https://openrouter.ai) API. Pick a model once, save your API
key, and chat — configuration lives in a plain `vibe.toml` next to where you run
it.

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
  - `POST /chat/completions` — send a chat request (used by `vibe chat`)

### Getting an API key

1. Sign in at <https://openrouter.ai>.
2. Create a key at <https://openrouter.ai/keys>.
3. Save it locally with:

   ```sh
   vibe api-key "sk-or-v1-..."
   ```

Some models are free; others are billed per token against your OpenRouter
credits. The model list shown by `vibe model` includes each model's pricing.

### Configuration

Settings are stored locally in `vibe.toml` in the current working directory:

```toml
api_key = "sk-or-v1-..."
model = "openai/gpt-4o-mini"
```

`vibe.toml` holds a secret and is listed in `.gitignore`. Both keys are written
by the commands below — you normally don't edit the file by hand.

## Commands

```
vibe <COMMAND> [ARGS]
```

| Command              | Description                                                    |
| -------------------- | -------------------------------------------------------------- |
| `vibe help`          | Show the help menu (`-h` and `--help` also work)               |
| `vibe api-key "<KEY>"` | Save your OpenRouter API key to `vibe.toml`                  |
| `vibe model`         | Fetch the available models and pick one interactively          |
| `vibe chat`          | Start an interactive chat with the selected model              |

### Examples

```sh
# Save your API key
vibe api-key "sk-or-v1-..."

# Choose a model from a live, searchable list
vibe model

# Start chatting
vibe chat
```

Inside `vibe chat`, type a message and press Enter. Type `exit` (or `quit`), or
press `Ctrl-D`, to leave.

### Typical first run

```sh
vibe api-key "sk-or-v1-..."
vibe model
vibe chat
```
