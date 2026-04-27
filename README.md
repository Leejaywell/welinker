# welinker

Rust implementation of a WeChat iLink to AI Agent bridge, based on the architecture in `../weclaw`.

## Features

- WeChat QR login through iLink.
- Long-poll message monitoring for saved accounts.
- Agent routing for ACP, CLI, and OpenAI-compatible HTTP agents.
- Built-in chat commands: `/info`, `/help`, `/new`, `/clear`, `/cwd`, `/agent message`.
- Built-in aliases: `/cc`, `/cx`, `/oc`, `/zc`, `/cs`, `/km`, `/gm`, `/ocd`, `/pi`, `/cp`, `/dr`, `/if`, `/kr`, `/qw`, `/hm`, `/hh`.
- Text and media sending through CLI and `POST /api/send`.
- Built-in WebUI for account status and manual sends.
- Inbound image, voice, file, and video saving when `save_dir` is configured.
- Multi-account listing, removal, and account-specific sending.
- Background process commands: `start`, `status`, `stop`, `restart`.

Runtime files are stored under `~/.welinker`:

- `~/.welinker/config.json`
- `~/.welinker/accounts/*.json`
- `~/.welinker/welinker.log`
- `~/.welinker/welinker.pid`

## Usage

```bash
cargo run -- login
cargo run -- start --foreground
cargo run -- start --foreground --web-only
cargo run -- start
cargo run -- status
cargo run -- send --to "user_id@im.wechat" --text "hello"
cargo run -- send --account "bot_id@im.bot" --to "user_id@im.wechat" --text "hello"
cargo run -- accounts list
cargo run -- accounts remove "bot_id@im.bot"
```

`start` auto-detects common local agents and writes the detected configuration to `~/.welinker/config.json`.

When `start --foreground` or the background service is running, open the WebUI at:

```text
http://127.0.0.1:18011/
```

Use `start --foreground --web-only` to open the WebUI and local agent chat without
triggering WeChat QR login. Sending WeChat messages still requires a logged-in
account.

## Config

```json
{
  "api_addr": "127.0.0.1:18011",
  "save_dir": "/Users/me/WeChat",
  "route_tag": "optional-sk-route-tag",
  "default_agent": "codex",
  "agents": {}
}
```

Environment overrides:

- `WELINKER_API_ADDR`
- `WELINKER_SAVE_DIR`
- `WELINKER_ROUTE_TAG`
- `WELINKER_DEFAULT_AGENT`
- `WELINKER_HERMES_HTTP_URL`
- `WELINKER_HERMES_HTTP_KEY`
- `WELINKER_HERMES_HTTP_MODEL`
- `WELINKER_ALLOW_REMOTE_API=1` permits binding `api_addr` to non-loopback addresses.

API endpoints:

- `GET /api/status`
- `GET /api/accounts`
- `POST /api/send` accepts `account_id` when multiple accounts are logged in.
- `POST /api/chat` sends a local message to the configured agent without WeChat login.
- `GET /api/config` reads `~/.welinker/config.json`.
- `PUT /api/config` validates and saves the full config JSON.

Config changes saved through the WebUI are written to disk immediately. Restart
Welinker to apply runtime changes such as default agent, aliases, or API address.

## Implementation Notes

### HTTP request handling

The built-in API server is intentionally small and dependency-light, but it still
handles request bodies by `Content-Length` instead of assuming a single TCP read
contains the full request. This matters for WebUI config saves and local chat
messages because browsers may split large JSON bodies across packets.

- Request headers are read until `\r\n\r\n`.
- `Content-Length` is parsed case-insensitively.
- The server reads exactly the declared body length before dispatching.
- Requests larger than 4 MiB are rejected with `413 Payload Too Large`.
- Incomplete or invalid bodies are rejected with `400 Bad Request`.

### Local API exposure

`GET /api/config`, `PUT /api/config`, and `POST /api/chat` expose sensitive local
capabilities: config files may include agent API keys, and chat requests can
invoke local agents. For that reason, the API server refuses to bind to
non-loopback addresses by default.

Safe default:

```json
{
  "api_addr": "127.0.0.1:18011"
}
```

Remote/LAN binding requires an explicit opt-in:

```bash
WELINKER_ALLOW_REMOTE_API=1 cargo run -- start --foreground --api-addr 0.0.0.0:18011
```

Only use remote binding on a trusted network or behind your own access control.

### Config save semantics

`PUT /api/config` validates and writes the full JSON config to
`~/.welinker/config.json`, then returns:

```json
{
  "reload_required": true
}
```

The running process does not hot-reload the saved config. Agent metadata,
aliases, default agent, save directory, and API address are initialized at
startup. Restart Welinker after saving config changes that affect runtime
behavior.

### API server startup failures

The API server runs in its own async task while message monitoring continues.
If binding fails, for example because the port is already in use, the failure is
logged as:

```text
api server stopped error=...
```

In foreground mode this is written through tracing. In background mode it appears
in `~/.welinker/welinker.log`.
