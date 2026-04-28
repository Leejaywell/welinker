# welinker 中文文档

welinker 是一个用 Rust 实现的 WeChat iLink 到 AI Agent 的桥接工具。它可以通过微信扫码登录，监听已保存账号的消息，并把消息路由到本地或 HTTP 形式的 AI Agent。

## 功能概览

- 通过 iLink 扫码登录微信账号。
- 对已保存账号进行长轮询消息监听。
- 支持 ACP、CLI、OpenAI 兼容 HTTP Agent 路由。
- 内置聊天命令：`/info`、`/help`、`/new`、`/clear`、`/cwd`、`/agent message`。
- 内置别名：`/cc`、`/cx`、`/oc`、`/zc`、`/cs`、`/km`、`/gm`、`/ocd`、`/pi`、`/cp`、`/dr`、`/if`、`/kr`、`/qw`、`/hm`、`/hh`。
- 支持通过 CLI 和 `POST /api/send` 发送文本与媒体消息。
- 内置 WebUI，可查看账号状态、手动发送消息、调整配置。
- 配置 `save_dir` 后可保存收到的图片、语音、文件和视频。
- 支持多账号列表、移除账号、指定账号发送消息。
- 支持后台进程命令：`start`、`status`、`stop`、`restart`。

运行时文件默认保存在 `~/.welinker`：

- `~/.welinker/config.json`
- `~/.welinker/accounts/*.json`
- `~/.welinker/welinker.log`
- `~/.welinker/welinker.pid`

## 安装

构建依赖：

- Rust/Cargo
- Node.js/npm，用于构建内置 WebUI

从当前源码目录安装：

```bash
./install.sh
```

默认安装位置是 `~/.local/bin/welinker`。如需指定安装前缀：

```bash
./install.sh --prefix /usr/local
```

使用 Homebrew 辅助本地安装：

```bash
scripts/install-homebrew.sh
```

该脚本会安装缺失的 `rust` 和 `node` formula，然后把 `welinker` 安装到 `$(brew --prefix)/bin`。

也可以从本仓库的 Homebrew formula 安装：

```bash
brew install --HEAD ./Formula/welinker.rb
```

## 快速开始

开发环境中可以直接使用 `cargo run`：

```bash
cargo run -- login
cargo run -- start --foreground
```

安装后可以直接使用 `welinker`：

```bash
welinker login
welinker start --foreground
```

首次启动时，如果本地没有已保存的微信账号，`start` 会自动进入登录流程。`start` 还会检测常见本地 Agent，并把检测到的配置写入 `~/.welinker/config.json`。

前台运行或后台服务启动后，可打开 WebUI：

```text
http://127.0.0.1:18011/
```

如果只想打开 WebUI 和本地 Agent 聊天，不触发微信扫码登录：

```bash
welinker start --foreground --web-only
```

注意：发送微信消息仍然需要至少一个已登录账号。

## 常用命令

```bash
welinker login
welinker start --foreground
welinker start --foreground --web-only
welinker start
welinker status
welinker stop
welinker restart
welinker version
welinker send --to "user_id@im.wechat" --text "hello"
welinker send --account "bot_id@im.bot" --to "user_id@im.wechat" --text "hello"
welinker accounts list
welinker accounts remove "bot_id@im.bot"
```

开发时也可以把上面的 `welinker` 替换成 `cargo run --`。

## 配置

主配置文件位于 `~/.welinker/config.json`。示例：

```json
{
  "api_addr": "127.0.0.1:18011",
  "save_dir": "/Users/me/WeChat",
  "route_tag": "optional-sk-route-tag",
  "default_agent": "gemini",
  "agents": {}
}
```

可用环境变量覆盖：

- `WELINKER_API_ADDR`
- `WELINKER_SAVE_DIR`
- `WELINKER_ROUTE_TAG`
- `WELINKER_DEFAULT_AGENT`
- `WELINKER_HERMES_HTTP_URL`
- `WELINKER_HERMES_HTTP_KEY`
- `WELINKER_HERMES_HTTP_MODEL`
- `WELINKER_ALLOW_REMOTE_API=1` 允许把 `api_addr` 绑定到非本机回环地址。

通过 WebUI 保存的配置会立即写入磁盘。默认 Agent、别名、API 地址、保存目录等运行时配置不会热重载，修改后需要重启 welinker 才会生效。

## HTTP API

内置 API 服务默认监听 `127.0.0.1:18011`。

- `GET /api/status`
- `GET /api/accounts`
- `POST /api/send`，多账号登录时可传入 `account_id`。
- `POST /api/chat`，不经过微信登录，直接向本地配置的 Agent 发送消息。
- `GET /api/config`，读取 `~/.welinker/config.json`。
- `PUT /api/config`，校验并保存完整配置 JSON。

`PUT /api/config` 成功后会返回：

```json
{
  "reload_required": true
}
```

## 安全说明

`GET /api/config`、`PUT /api/config` 和 `POST /api/chat` 会暴露敏感的本地能力：配置文件可能包含 Agent API key，聊天接口也可以调用本地 Agent。因此 API 服务默认拒绝绑定到非回环地址。

安全默认配置：

```json
{
  "api_addr": "127.0.0.1:18011"
}
```

如确需在局域网或远程环境访问，需要显式开启：

```bash
WELINKER_ALLOW_REMOTE_API=1 welinker start --foreground --api-addr 0.0.0.0:18011
```

只应在可信网络或自有访问控制之后使用远程绑定。

## 排错

后台模式日志位于：

```text
~/.welinker/welinker.log
```

如果端口已被占用或 API 服务绑定失败，前台模式会在 tracing 日志中输出错误；后台模式会写入 `~/.welinker/welinker.log`，常见日志格式如下：

```text
api server stopped error=...
```

如果保存 WebUI 配置或本地聊天失败，先检查 API 是否仍在运行：

```bash
welinker status
```

然后确认 `api_addr` 是否仍为本机可访问地址，并在修改运行时配置后重启 welinker。
