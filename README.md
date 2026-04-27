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
