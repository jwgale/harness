# Harness

A CLI tool that orchestrates **planner -> builder -> evaluator** loops using subscription CLI tools (Claude Code, Codex). Zero API cost.

Inspired by [Anthropic's harness architecture](https://www.anthropic.com/engineering/harness-design-long-running-apps) for long-running application development with Opus 4.6.

## v0.11.2 Release Notes

Harness v0.11.2 completes the Telegram progress protocol with clean lifecycle management.

**New in v0.11.2:**
- **Listener Shutdown Signal** — `ListenerHandle` with `Drop` impl that signals shutdown via `AtomicBool`; no more hard 10-minute timeout. Socket cleaned up automatically when the bridge is done
- **Priority Progress Buffer** — EVENT/DONE lines always kept; only STDOUT lines are evicted when the buffer is full. Verbose agents can't push out lifecycle events
- **Configurable Buffer Size** — `progress_buffer_size` in `[bridge]` config (default: 50 lines)
- **27 Unit Tests + 31 Integration Tests** — including priority buffer and shutdown tests

**v0.11.1:**
- **Event-Driven Telegram Updates** — sends immediately on significant events instead of fixed timer; rate-limited to ~1 msg per 6 seconds
- **Multi-Client Socket** — progress listener now accepts multiple concurrent connections
- **Listener-Side Audit Trail** — `progress.log` written only by the socket listener
- **Smart Batching** — STDOUT lines batched and included in next significant-event send

**v0.11.0:**
- **Unix Socket Progress** — bridge creates `.harness/progress.sock`; runner streams `EVENT:`, `STDOUT:`, and `DONE:` messages in real time (sub-second latency)
- **Raw Agent Stdout** — non-TUI runner now uses streaming mode when progress socket is available, forwarding every agent output line with agent-name prefix
- **Live Stdout in Telegram** — `/run --wait` shows raw agent output lines as they happen
- **Automatic Fallback** — if socket creation fails, falls back to file-based `progress.log` polling

**v0.10.3:**
- **Real-time Progress Protocol** — multi-agent runner writes timestamped progress to `.harness/progress.log` as agents execute; bridge polls this for live updates
- **Live Progress in Telegram** — `/run --wait` now shows real-time agent step starts, completions, verdicts, loop iterations, and parallel batch status
- **Policy Endpoint Optional** — new `require_policy_endpoint = false` (default) makes `_policy` vault endpoint optional with clear docs; set to `true` only if your vault supports it
- **Improved Error Messages** — policy denial messages now include hints when vault is misconfigured

**v0.10.2:**
- **Configurable Workflow Timeout** — set `workflow_timeout_minutes` globally in `[bridge]` config or per-workflow in TOML (default: 30m)
- **Strict Policy Mode** — `strict_policy_mode = true` in `[bridge]` config denies commands when vault is unreachable or policies are missing
- **Rich Progress Updates** — `/run --wait` now reads per-agent status, feedback rounds, and multi-agent output for detailed progress messages
- **Agent Completion Summary** — workflow results include per-agent output summaries

**v0.10.1:**
- **Workflow Completion Callback** — `/run <workflow>` sends results back to Telegram when the workflow finishes (verdict, timing, evaluation summary)
- **`/run --wait` Mode** — blocks with periodic progress updates until the workflow completes
- **Vault Policy Authorization** — each bridge command checks `harness:bridge:telegram:<cmd>` policy before executing; graceful denial on failure
- **Robust Markdown Escaping** — `escape_markdown()` helper for safe Telegram MarkdownV1 output with automatic plain-text fallback
- **Active Workspace Discovery** — `/run` auto-finds a registered workspace with `.harness/` to run workflows in

**v0.10.0:**
- **Telegram Command Bridge** — `harness bridge telegram start/status/stop` for chat-based control
- **Bot Commands** — `/run`, `/status`, `/agent list`, `/vault status` via Telegram
- **Systemd-Managed** — bridge runs as a background service with auto-restart
- **Vault-Only Credentials** — bot token and chat ID pulled exclusively from SanctumAI vault
- **SCL Recording** — every bridge command and response logged to Shared Context Layer

**v0.9.0:**
- **SanctumAI Vault** — `harness vault init/status/add/list` for Ed25519-authenticated credential management
- **Vault-Aware Notifications** — Slack, Telegram, email, and webhook credentials auto-resolve from vault before config fallback
- **Ed25519 Agent Identity** — auto-generated signing key for vault authentication
- **28 Integration Tests** — including vault init and status tests

**v0.8.0:**
- **Sequential Streaming** — all multi-agent steps stream live to TUI
- **Agent Legend** — color-coded agent list with filter key hints
- **Multi-Agent Header** — "Harness Multi-Agent" title when agents active

**v0.4.0:**
- **Multi-Agent Orchestration** — `harness agent add/list/remove` for TOML-based agent definitions
- **Named Workflows** — `harness run --workflow <name>` runs TOML-defined agent sequences
- **Agent CLI** — `harness run --agents planner,builder,evaluator` for ad-hoc multi-agent runs
- **Custom Roles** — define specialized agents (security reviewer, documentation writer, etc.)
- **SCL Recording** — all agent runs, steps, and outcomes automatically recorded

**v0.3.0:**
- **Custom Evaluators** — `harness evaluator list/use` to switch between evaluation strategies per workspace
- **Built-in Strategies** — `default` (prompt-based), `playwright-mcp` (browser interaction), `curl` (HTTP health checks)
- **External Notifications** — Slack, Telegram, email, and webhook notification plugins that fire on eval pass/fail and schedule completion

**v0.2.0 foundation:**
- **Core Orchestrator** — plan -> build -> evaluate -> revise loop with TUI, prompt overrides, streaming output, and verdict parsing
- **Local Install** — one-liner `curl` installer, XDG-compliant directory layout, `cargo install --path .`
- **Persistent Daemon** — systemd user service with `harness daemon start/stop/status/logs`
- **Plugin System** — TOML-based plugins with 6 lifecycle hooks, configurable timeouts, hot-reload
- **Workspace Management** — `harness workspace register/list/remove` with inotify file watching
- **Scheduled Tasks** — cron-style scheduling with deduplication, local timezone, execution history, manual triggers
- **Shared Context Layer** — built-in MCP integration with direct HTTP client (0.1s queries), auto-recording of lifecycle events
- **Mock Backend** — `--backend mock` for instant testing without real Claude/Codex

## Installation

### One-liner (Linux)

```bash
curl -fsSL https://raw.githubusercontent.com/jwgale/harness/main/install.sh | sh
```

This installs the `harness` binary to `~/.local/bin/`. If no pre-built release exists yet, it builds from source (requires Rust).

### From source

```bash
git clone https://github.com/jwgale/harness.git
cd harness
cargo install --path .
```

Requires Rust 2024 edition (1.85+).

### Verify

```bash
harness --version
harness doctor
```

## Prerequisites

- **Claude Code CLI** authenticated (Claude Max / Team subscription), OR
- **Codex CLI** authenticated (ChatGPT Pro subscription)

## Quick Start

```bash
# 1. Initialize a new project harness
harness init "Build a CLI todo app in Rust with SQLite storage"

# 2. Run the full automated loop (launches TUI, defaults to Codex)
harness run

# Or run with human review pauses (plain text mode)
harness run --pause-after-plan --pause-after-eval

# Or run individual phases
harness plan
# Review/edit .harness/spec.md
harness build
harness evaluate

# Override the default backend when needed
harness run --backend claude
```

## Commands

| Command | Description |
|---------|-------------|
| `harness init <goal>` | Create `.harness/` directory, write goal, generate config |
| `harness plan [--backend claude\|codex]` | Run planner to generate `spec.md` |
| `harness build [--backend claude\|codex]` | Run builder to implement the spec |
| `harness evaluate [--backend claude\|codex]` | Run evaluator to assess the build |
| `harness run [options]` | Full automated loop: plan -> build -> evaluate -> revise |
| `harness status` | Print current harness state |
| `harness doctor [--deep]` | Diagnose backend, toolchain, config, and quality-gate readiness |
| `harness reset` | Generate `handoff.md` for context reset |
| `harness feedback` | Show latest evaluator feedback |
| `harness daemon <action>` | Manage persistent daemon (start/stop/status/logs) |
| `harness plugin list` | List installed plugins |
| `harness workspace register [path]` | Register a project for daemon monitoring (default: `.`) |
| `harness workspace list` | List registered workspaces |
| `harness workspace remove <name>` | Remove a registered workspace |
| `harness schedule add <name> "<cron>" "<cmd>"` | Add a cron-style scheduled task |
| `harness schedule list` | List scheduled tasks |
| `harness schedule remove <name>` | Remove a scheduled task |
| `harness schedule run <name>` | Manually trigger a schedule now |
| `harness schedule history [--limit N]` | Show schedule execution history |
| `harness context status` | Show SCL connection status |
| `harness context query "<text>"` | Query the Shared Context Layer |
| `harness context record <type> "<text>"` | Record an entry to SCL |
| `harness evaluator list` | List available evaluator strategies |
| `harness evaluator use <name>` | Set evaluator strategy for this workspace |
| `harness agent list` | List defined agents |
| `harness agent add <name> --role <role> --backend <backend>` | Create a new agent definition |
| `harness agent remove <name>` | Remove an agent definition |
| `harness workflow list` | List defined workflows |
| `harness workflow validate <name>` | Validate a workflow definition |
| `harness vault init` | Initialize vault config and generate signing key |
| `harness vault status` | Show vault connection status |
| `harness vault add <name>` | Store a credential in the vault |
| `harness vault list` | List credentials in the vault |
| `harness bridge telegram start` | Start the Telegram bot bridge (systemd-managed) |
| `harness bridge telegram status` | Show bridge status and credential health |
| `harness bridge telegram stop` | Stop the Telegram bridge |

### `harness run` options

- `--backend claude|codex` — which CLI backend to use
- `--max-rounds N` — maximum evaluation/revision rounds (default: 3)
- `--pause-after-plan` — pause for human review after planning
- `--pause-after-eval` — pause for human review after each evaluation
- `--no-tui` — disable TUI, use plain text output
- `--agents planner,builder,evaluator` — run named agents sequentially (multi-agent mode)
- `--workflow <name>` — run a named workflow from `~/.config/harness/workflows/`

Codex builder runs use `codex exec --full-auto` by default. If you are already
running Harness inside an external sandbox and want Codex to bypass its own
approval and sandbox prompts, set `HARNESS_CODEX_DANGEROUS=1`.

## How the Loop Works

```
1. PLAN:     Goal -> Planner -> spec.md
2. BUILD:    spec.md + feedback -> Builder -> working code + status.md
3. EVALUATE: spec.md + status.md + code -> Evaluator -> evaluation.md
4. DECISION:
   - PASS:   Done. Exit 0.
   - REVISE: Loop back to BUILD with feedback (up to max rounds).
   - FAIL:   Exit 1.
```

The planner, builder, and evaluator are separate agent sessions. They communicate through files in `.harness/`. The builder has full file system and git access — it writes code directly.

## TUI

By default, `harness run` launches a split-pane TUI showing:
- **Left pane**: project info, current phase, elapsed time, feature checklist, evaluation scores
- **Right pane**: live streaming output from the current agent with syntax highlighting

Keyboard shortcuts:
- `q` — quit
- `f` / `End` — toggle follow mode
- `j`/`k` or arrow keys — scroll
- `PgUp`/`PgDn` — page scroll
- `Tab` — toggle split/full-width output
- `1`/`2`/`3` — switch view mode (split/output/status)
- `` ` `` — cycle agent output filter (All → Agent 1 → Agent 2 → ...)
- `4`/`5`/`6`/`7` — jump to All / Agent 1 / Agent 2 / Agent 3 filter

## Artifacts

Per-project state lives in `.harness/` (inside the project directory):

```
.harness/
  config.json          # Backend, model, project name, max rounds
  goal.md              # Original goal text
  spec.md              # Planner output
  status.md            # Builder-maintained progress
  evaluation.md        # Latest evaluator assessment
  handoff.md           # Context reset brief (generated by `harness reset`)
  endpoints.json       # HTTP endpoints for curl evaluator (optional)
  prompts/             # Custom prompt overrides (optional)
  feedback/
    round-001.md       # Per-round evaluator feedback
  runs/
    run-001.json       # Run metadata (timing, outcome)
  agents/              # Isolated parallel agent outputs
    <agent-name>/
      status.md        # Agent-specific output
```

## Directory Layout (XDG)

Global state follows XDG conventions:

| Path | Purpose |
|------|---------|
| `~/.config/harness/` | Global config, plugin manifests |
| `~/.config/harness/plugins/` | Plugin TOML files |
| `~/.config/harness/agents/` | Agent definition TOML files |
| `~/.config/harness/workflows/` | Workflow definition TOML files |
| `~/.local/share/harness/` | Persistent data (daemon PID, run history) |
| `~/.cache/harness/` | Temporary cache |

## Customizing Prompts

The binary embeds default prompt templates. To override them, place custom versions in `.harness/prompts/`:

- `.harness/prompts/planner.md`
- `.harness/prompts/builder.md`
- `.harness/prompts/evaluator.md`

If a file exists in `.harness/prompts/`, it takes precedence over the embedded default.

## Daemon

The harness daemon runs as a systemd user service for persistent background agent orchestration.

```bash
harness daemon start    # Install and start the systemd user service
harness daemon status   # Check if the daemon is running
harness daemon logs     # View recent daemon logs
harness daemon stop     # Stop and disable the service
```

The daemon uses real-time file watching (inotify) to monitor registered workspaces for `.harness/` artifact changes and fires plugin hooks when files are modified.

### Workspaces

Register project directories so the daemon can watch them:

```bash
harness workspace register ~/projects/my-app    # Register a project
harness workspace list                           # List all registered workspaces
harness workspace remove my-app                  # Remove a workspace
```

When the daemon is running and a workspace's `spec.md`, `status.md`, or `evaluation.md` changes, the corresponding plugin hooks fire automatically.

## Plugins

Plugins are TOML manifests placed in `~/.config/harness/plugins/`. They declare lifecycle hooks that fire during the plan/build/evaluate loop.

```toml
# ~/.config/harness/plugins/my-plugin.toml
name = "my-plugin"
description = "Run tests after every build"
version = "0.1.0"
timeout_seconds = 60  # per-hook timeout (default: 30s)

[hooks]
before_plan = "echo 'Planning...'"
after_build = "cargo test"
before_evaluate = "cargo clippy"
```

Hooks that exceed their timeout are killed automatically.

Available hook points: `before_plan`, `after_plan`, `before_build`, `after_build`, `before_evaluate`, `after_evaluate`.

List installed plugins:
```bash
harness plugin list
```

Hooks execute as shell commands with environment variables:

| Variable | Description |
|----------|-------------|
| `HARNESS_HOOK` | Which hook is firing (e.g. `after_build`) |
| `HARNESS_PLUGIN` | Plugin name |
| `HARNESS_PROJECT` | Project directory name |
| `HARNESS_DIR` | Path to `.harness/` directory |
| `HARNESS_PLUGINS_DIR` | Path to plugins directory |

Hooks fire during `harness plan`, `harness build`, `harness evaluate`, and `harness run`.

### Writing Your First Plugin

1. Create a TOML file in `~/.config/harness/plugins/`:

```toml
# ~/.config/harness/plugins/my-tests.toml
name = "my-tests"
description = "Run cargo test after every build"
version = "0.1.0"
timeout_seconds = 120  # long timeout for test suites

[hooks]
after_build = "cargo test 2>&1 || echo 'Tests failed!'"
```

2. Verify it's loaded: `harness plugin list`
3. Run a build — the hook will execute automatically.

See `plugins/example.toml` in the repo for a complete example with all hook points.

The daemon hot-reloads plugins automatically — add, edit, or remove TOML files and the daemon picks up changes instantly.

## Scheduled Tasks

Schedule commands to run on a cron-style schedule via the daemon:

```bash
# Run a full harness loop every day at 8:30 AM UTC
harness schedule add daily-build "30 8 * * *" "cd ~/projects/my-app && harness run --backend claude --no-tui"

# Run tests every hour
harness schedule add hourly-tests "0 * * * *" "cd ~/projects/my-app && cargo test"

# List schedules
harness schedule list

# Remove a schedule
harness schedule remove daily-build
```

Schedules are stored as plugin TOML files and executed by the daemon. The daemon must be running for schedules to fire.

Cron format: `minute hour day-of-month month day-of-week` (local time) with support for `*`, `*/N`, ranges (`1-5`), and lists (`1,5,10`).

### Reliability

- **Deduplication** — each schedule tracks its last execution minute; daemon restarts won't fire the same schedule twice in the same minute
- **Local timezone** — cron expressions match against your local time, not UTC
- **Execution history** — every execution is logged to `~/.local/share/harness/schedule-history.jsonl` (entries older than 60 days are automatically pruned)

View recent executions:
```bash
harness schedule history           # last 20 entries
harness schedule history --limit 5 # last 5 entries
```

### Mock Backend

For testing, use `--backend mock` to skip real Claude/Codex invocations:
```bash
harness plan --backend mock    # instant mock response
harness run --backend mock     # full loop with mock responses
```

## Shared Context Layer (SCL)

Harness integrates with the [Shared Context Layer](https://github.com/jwgale/shared-context-layer) MCP server so every planner, builder, and evaluator session automatically has access to long-term memory, architectural decisions, and cross-project learnings.

When the SCL is enabled and reachable, harness automatically passes `--mcp-config` to all Claude Code invocations. No manual MCP configuration needed.

```bash
# Check connection
harness context status

# Query shared context (fast — direct MCP, no Claude session)
harness context query "recent architectural decisions"

# Record a decision
harness context record decision "Chose SQLite over Postgres for local-first storage"
```

Valid record kinds: `architecture`, `decision`, `convention`, `active_work`, `insight`, `gotcha`.

### Automatic Lifecycle Recording

When `auto_record` is enabled (default), harness automatically records key events to SCL:
- After planning: "Plan completed for project X"
- After each build round: "Build round N completed"
- After evaluation: verdict + scores

This gives every future Claude Code session access to the full history of your harness runs. Disable with `auto_record = false`.

### Configuration

```toml
# ~/.config/harness/config.toml
[shared_context]
enabled = true
url = "http://127.0.0.1:3100/mcp"
auto_record = true    # record plan/build/evaluate events automatically
```

Set `enabled = false` to disable SCL integration entirely. If the SCL server is unreachable, harness silently falls back to running without it. Health checks are cached for 60 seconds.

## Custom Evaluators

Harness supports pluggable evaluator strategies that change how your build is assessed.

### Available Strategies

| Strategy | Description |
|----------|-------------|
| `default` | Prompt-based evaluation via CLI backend (Claude/Codex/mock) |
| `playwright-mcp` | Uses Playwright MCP to interact with the running app in a browser |
| `curl` | Simple HTTP health-check evaluation for APIs |

### Usage

```bash
# List available strategies
harness evaluator list

# Set strategy for this workspace
harness evaluator use playwright-mcp

# Run evaluation (uses configured strategy)
harness evaluate --backend claude
```

The strategy is stored in `.harness/config.json` and respected by both `harness evaluate` and `harness run`.

### Playwright MCP Strategy

When set, the evaluator is instructed to use the Playwright MCP tool to:
1. Launch and navigate to your running application
2. Interact with the UI (click buttons, fill forms, navigate)
3. Verify core user flows end-to-end
4. Take screenshots of failures

Falls back to code inspection if the app isn't a web application.

### Curl Strategy

Checks HTTP endpoints before running the prompt-based evaluation. Configure endpoints in `.harness/endpoints.json`:

```json
["http://localhost:3000", "http://localhost:3000/api/health", "http://localhost:3000/api/status"]
```

Health check results (2xx = OK, other = FAILED, timeout = UNREACHABLE) are prepended to the evaluator prompt so the LLM can factor them into scoring.

## External Notifications

Notification plugins fire on evaluator and schedule lifecycle events. They use the same plugin directory (`~/.config/harness/plugins/`) as regular plugins.

### Notification Events

| Event | Fires when |
|-------|-----------|
| `on_eval_pass` | Evaluator returns PASS verdict |
| `on_eval_fail` | Evaluator returns FAIL verdict |
| `on_eval_revise` | Evaluator returns REVISE verdict |
| `on_schedule_complete` | A scheduled task finishes (success or failure) |

### Strategies

**Slack** — POST to an incoming webhook:
```toml
# ~/.config/harness/plugins/notify-slack.toml
name = "notify-slack"
description = "Slack notifications"

[notifications]
strategy = "slack"
url = "https://hooks.slack.com/services/YOUR/WEBHOOK/URL"
events = ["on_eval_pass", "on_eval_fail"]
```

**Telegram** — Send via Bot API:
```toml
[notifications]
strategy = "telegram"
bot_token = "YOUR_BOT_TOKEN"
chat_id = "YOUR_CHAT_ID"
```

**Email** — Send via local `mail` command:
```toml
[notifications]
strategy = "email"
to = "you@example.com"
from = "harness@localhost"
```

**Webhook** — POST JSON to any URL:
```toml
[notifications]
strategy = "webhook"
url = "https://your-server.com/harness-webhook"
```

Omit the `events` array to fire on all events. Notification plugins can coexist with regular hook plugins in the same TOML file.

Example notification plugin templates are in the `plugins/` directory of this repo.

## Credential Management (Powered by SanctumAI)

Harness integrates with [SanctumAI](https://sanctumai.dev) for secure credential management. Instead of storing secrets in plaintext TOML config, credentials are stored in an encrypted vault with Ed25519-authenticated access.

### Setup

```bash
# 1. Install and start a SanctumAI vault (see sanctumai.dev)
# 2. Initialize harness vault integration
harness vault init

# 3. Register the agent's public key with your vault
sanctum agent register harness --pubkey <key shown by init>

# 4. Store credentials
harness vault add notifications/slack/webhook-url
harness vault add notifications/telegram/bot-token
harness vault add notifications/telegram/chat-id
```

### Configuration

Vault settings in `~/.config/harness/config.toml`:

```toml
[vault]
enabled = true
addr = "127.0.0.1:7600"    # TCP address or Unix socket path
agent_name = "harness"
```

### How It Works

When sending notifications, harness checks the vault first for credentials matching well-known paths:

| Vault Path | Used By |
|------------|---------|
| `notifications/slack/webhook-url` | Slack notifications |
| `notifications/telegram/bot-token` | Telegram notifications |
| `notifications/telegram/chat-id` | Telegram notifications |
| `notifications/email/to` | Email notifications |
| `notifications/webhook/url` | Webhook notifications |

If the vault is unreachable or the credential doesn't exist, harness falls back to plaintext values in the plugin TOML. This means vault integration is **opt-in** — existing notification plugins continue to work without a vault.

### Commands

```bash
harness vault init       # Generate signing key, add [vault] to config
harness vault status     # Show connection status and credential count
harness vault add <path> # Store a credential (reads value from stdin)
harness vault list       # List available credentials
```

### Security Model

- **Ed25519 authentication** — agent identity is a signing key stored in `~/.local/share/harness/vault-key.bin` (mode 0600)
- **Challenge-response** — vault issues a random challenge; harness signs it to prove identity
- **Lease-based access** — retrieved credentials have a 5-minute TTL
- **Audit trail** — all credential access is logged by the vault

## Telegram Command Bridge

Control Harness from your phone or a Telegram group via a bot bridge.

### Setup

```bash
# 1. Create a Telegram bot via @BotFather and note the token
# 2. Get your chat/group ID (send a message, then check https://api.telegram.org/bot<token>/getUpdates)

# 3. Store credentials in the vault
harness vault add notifications/telegram/bot-token
# (paste your bot token when prompted)
harness vault add notifications/telegram/chat-id
# (paste your chat ID when prompted)

# 4. Start the bridge
harness bridge telegram start
```

### Bot Commands

| Command | Description |
|---------|-------------|
| `/run <workflow>` | Start a workflow; sends result on completion |
| `/run <workflow> --wait` | Start a workflow with periodic progress updates |
| `/status` | Show workspaces, schedules, workflows, daemon/bridge state |
| `/agent list` | List defined agents |
| `/vault status` | Show vault connection health |
| `/help` | List available commands |

### `/run` Examples

```
/run standard
  → "Workflow 'standard' started (PID 12345) in /home/user/projects/myapp
     You'll get a result when it finishes (timeout: 30m)."
  ... (workflow runs in background) ...
  → "Workflow 'standard' completed (142s)
     Verdict: PASS

     Agents:
       my-builder: output (2048 bytes)"

/run standard --wait
  → "Workflow 'standard' started (PID 12345)
     Waiting for completion (timeout: 30m)..."
  → "Workflow 'standard' running... (30s)

     Step 1/3: agent 'my-planner' started
     [my-planner] Analyzing project goal...
     [my-planner] Writing spec.md with 5 features
     Planner 'my-planner' done -- spec.md written
     Step 2/3: agent 'my-builder' started
     [my-builder] Implementing authentication module
     [my-builder] Writing src/auth.rs (142 lines)"
  → "Workflow 'standard' completed (142s)
     Verdict: PASS

     ## Evaluation Summary
     All acceptance criteria met.

     Agents:
       my-planner: output (512 bytes)
       my-builder: output (4096 bytes)"
```

The default mode (no `--wait`) returns immediately and sends results asynchronously when the workflow finishes. The `--wait` mode streams live agent stdout via a Unix domain socket (`.harness/progress.sock`) and sends Telegram updates every 30 seconds with the latest output lines. If socket creation fails, it falls back to polling `progress.log`.

### Permission System

All bridge commands check SanctumAI vault policies before executing:

| Policy | Governs |
|--------|---------|
| `harness:bridge:telegram:run` | `/run` command |
| `harness:bridge:telegram:status` | `/status` command |
| `harness:bridge:telegram:agent` | `/agent` commands |
| `harness:bridge:telegram:vault` | `/vault` commands |

The `_policy` vault endpoint is **optional** -- most SanctumAI vaults don't implement it yet. By default, if the vault is unreachable or policies aren't configured, commands are **allowed**.

```toml
# ~/.config/harness/config.toml
[bridge]
strict_policy_mode = true           # deny when vault unreachable or policy missing
require_policy_endpoint = false     # true = require vault to implement _policy (default: false)
workflow_timeout_minutes = 45       # max runtime for bridge-triggered workflows (default: 30)
```

| Flag | Default | Effect |
|------|---------|--------|
| `strict_policy_mode` | `false` | When `true`, missing policy responses deny by default |
| `require_policy_endpoint` | `false` | When `true`, vault must implement `_policy` or commands are denied |
| `workflow_timeout_minutes` | `30` | Max runtime for `/run` triggered workflows |
| `progress_buffer_size` | `50` | Max lines in progress buffer (EVENT lines prioritized over STDOUT) |

Per-workflow timeouts override the global setting:

```toml
# ~/.config/harness/workflows/long-build.toml
name = "long-build"
timeout_minutes = 60    # overrides global bridge config for this workflow
```

When a policy denies a command, the bot replies with a clear denial message including the policy name and a hint if misconfigured.

### Management

```bash
harness bridge telegram start    # Start bridge (validates credentials first)
harness bridge telegram status   # Check bridge + credential health
harness bridge telegram stop     # Stop and disable the bridge
```

The bridge runs as a systemd user service (`harness-telegram`) with automatic restart on failure. It uses long-polling (no webhooks/ports needed).

All commands and responses are recorded to the Shared Context Layer for audit and cross-session visibility. Markdown output is escaped for safe rendering, with automatic plain-text fallback on parse errors.

### Notifications

When the bridge is running, notification plugins configured for Telegram will send to the same chat. The bridge bot and notification system share the same vault credentials.

## Multi-Agent Orchestration

Define named agents and compose them into workflows for flexible multi-agent runs.

### Defining Agents

Agents are TOML files in `~/.config/harness/agents/`:

```toml
# ~/.config/harness/agents/my-planner.toml
name = "my-planner"
role = "planner"
backend = "codex"
description = "Plans the build from the project goal"
# model = "default"            # optional model override
# timeout_seconds = 600        # optional timeout
# prompt_template = "..."      # inline prompt or file path
# tools = ["git", "grep"]      # optional tool list
# specializations = ["frontend", "react"]   # optional selector tags
# context_scopes = ["ui", "web"]            # optional SCL scopes
# default_for = ["frontend"]                # optional @selector default
```

Valid roles: `planner`, `builder`, `evaluator`, `custom`.

```bash
# Create agents via CLI
harness agent add my-planner --role planner --backend codex
harness agent add my-builder --role builder --backend codex \
  --specializations frontend,react --default-for frontend
harness agent add my-evaluator --role evaluator --backend codex

# List defined agents
harness agent list

# Remove an agent
harness agent remove my-planner
```

### Running with Named Agents

Use `--agents` to run a comma-separated list of agents sequentially:

```bash
harness run --agents my-planner,my-builder,my-evaluator --no-tui
```

Each agent runs in order. Planner agents write `spec.md`, builders write `status.md`, evaluators write `evaluation.md` and return a verdict. Custom agents write their output to `.harness/agent-<name>.md`.

### Specialized Agents

Agents can advertise domain tags and selector aliases:

```toml
name = "frontend-builder"
role = "builder"
backend = "codex"
specializations = ["frontend", "react"]
context_scopes = ["ui", "web"]
default_for = ["frontend"]
```

Workflows and `--agents` accept `@selector` references. Harness resolves `@frontend` to the single matching agent, preferring an agent that declares `default_for = ["frontend"]`. If multiple agents match and none is the default, validation fails with an ambiguity error.

```bash
harness run --agents @frontend,my-evaluator --no-tui
```

Workflow steps can require specializations:

```toml
[[steps]]
agent = "@frontend"
requires = ["frontend"]
```

Validation checks that the resolved agent satisfies every `requires` tag before the workflow runs.

### Parallel Execution

Run agents concurrently with `--parallel`:

```bash
harness run --agents frontend-builder,backend-builder --parallel --no-tui
```

In workflows, mark steps as `parallel: true` to run them concurrently:

```toml
[[steps]]
agent = "frontend-builder"
parallel = true

[[steps]]
agent = "backend-builder"
parallel = true

[[steps]]
agent = "my-evaluator"
# Not parallel — runs after both builders complete
```

Adjacent `parallel: true` steps form a batch that executes concurrently. Non-parallel steps run after the batch completes.

**Artifact isolation:** Each parallel agent writes to its own namespace (`.harness/agents/<name>/`) to prevent write conflicts. On completion, outputs are automatically merged into the shared location with agent headers. Plugin hooks fire before and after each parallel batch.

**Real-time streaming:** In TUI mode, parallel agents stream output live. Lines are prefixed with `[agent-name]` and color-coded per agent. Press backtick (`` ` ``) to cycle between viewing all output or filtering to a single agent. Keys `4`-`7` jump directly to All / Agent 1 / Agent 2 / Agent 3.

**Artifact overrides:** Use `output_artifact` in workflow steps to control where output is written:
```toml
[[steps]]
agent = "frontend-builder"
parallel = true
output_artifact = "frontend-status.md"
```

### Iterative Build-Evaluate Loops

Add `loop_until = "pass"` to a builder step to create an automatic revision loop:

```toml
[[steps]]
agent = "my-planner"

[[steps]]
agent = "my-builder"
loop_until = "pass"
max_rounds = 5

[[steps]]
agent = "my-evaluator"
```

The builder and evaluator will iterate until the evaluator returns PASS or `max_rounds` is exhausted. This brings the same revision loop from `harness run` into multi-agent workflows.

### Defining Workflows

Workflows are TOML files in `~/.config/harness/workflows/` that define a sequence of agent steps:

```toml
# ~/.config/harness/workflows/standard.toml
name = "standard"
description = "Standard plan-build-evaluate with revision loop"
max_rounds = 3

[[steps]]
agent = "my-planner"

[[steps]]
agent = "my-builder"
loop_until = "pass"

[[steps]]
agent = "my-evaluator"
```

Run a workflow:
```bash
harness run --workflow standard
```

Steps can override prompts:
```toml
[[steps]]
agent = "my-evaluator"
prompt = "Focus only on security issues in this evaluation."
```

### Workflow Validation

Validate workflows before running to catch errors early:

```bash
# Validate a specific workflow
harness workflow validate standard

# List all workflows
harness workflow list
```

Validation checks:
- All referenced agents exist
- `@selector` agent references resolve to a single concrete agent
- Agent backends and roles are valid
- Required step specializations are satisfied
- `loop_until` steps have a subsequent evaluator
- No structural errors

### Custom Agents

Create agents with the `custom` role for specialized tasks (security review, documentation, etc.):

```toml
name = "security-reviewer"
role = "custom"
backend = "codex"
prompt_template = "Review the codebase for OWASP Top 10 vulnerabilities..."
```

Custom agents receive the project goal and spec as context, plus any custom prompt template you provide.

### SCL Integration

Multi-agent runs automatically record to the Shared Context Layer:
- Which agents were used in each run
- Each agent step completion with status
- Parallel execution groups (start/end)
- Iterative loop iterations
- Final run outcome

Specialized agents also query SCL before default prompts are sent when `context_scopes` or `specializations` are present and SCL is reachable. The retrieved cross-project context is prepended to the prompt with the agent identity. If SCL is disabled or unreachable, the run continues without injected context.

Example agent and workflow templates are in the `examples/` directory of this repo.

## Configuration

### Global Config

`~/.config/harness/config.toml` is created automatically on first run:

```toml
[shared_context]
enabled = true
url = "http://127.0.0.1:3100/mcp"
```

### Project Config

`.harness/config.json` is created by `harness init`:

```json
{
  "backend": "codex",
  "model": "default",
  "project_name": "my-project",
  "max_eval_rounds": 3,
  "builder_timeout_seconds": 1800,
  "evaluator_timeout_seconds": 600,
  "evaluator_strategy": "default",
  "created_at": "2026-04-01T12:00:00Z"
}
```

## Roadmap to OpenClaw-Style Harness

Harness is evolving from a thin orchestrator into a full local-first agent platform.

**Phase 1: Core Orchestrator (done)**
- Plan -> build -> evaluate -> revise loop
- TUI with live streaming output
- Prompt override system
- Installable binary

**Phase 2: Local Install Polish (done)**
- One-liner installer with GitHub Releases
- XDG-compliant directory layout
- Plugin/skill system foundation
- Daemon skeleton

**Phase 3: Persistent Daemon + Plugin Hooks (done)**
- Systemd user service daemon (`harness daemon start/stop/status/logs`)
- Plugin hook points wired into plan/build/evaluate lifecycle
- `harness plugin list` with hook counts
- Hook discovery and logging (execution in Phase 4)

**Phase 4: Executable Hooks + Active Daemon (done)**
- Plugin hooks execute shell commands with env vars
- Hooks fire in all commands (plan, build, evaluate, run)
- Example plugin included in repo

**Phase 5: Workspace Management + Hook Robustness (done)**
- `harness workspace register/list/remove` for daemon monitoring
- Real-time file watching via inotify (replaces polling)
- Configurable per-plugin hook timeout (default 30s, kills on exceed)
- Hook output routed to TUI live output panel

**Phase 6: Hot-Reload + Scheduled Tasks + Tests (done)**
- Daemon hot-reloads plugins on TOML file changes
- `harness workspace register` defaults to current dir
- `harness schedule add/list/remove` with cron-style expressions
- Integration test suite

**Phase 7: Schedule Reliability + Test Speed (done)**
- Schedule deduplication, local timezone, execution history
- Mock backend for instant testing

**Phase 8: Final Polish + v0.2.0 (done)**
- Atomic state writes, manual schedule trigger, 60-day history pruning

**Phase 9: Shared Context Layer Integration (done)**
- Built-in MCP config injection for all Claude Code sessions
- `harness context status/query/record` commands
- Auto-generated global config with SCL settings
- Graceful fallback when SCL is unreachable

**Phase 10: SCL Polish + Auto-Recording (done)**
- Direct MCP HTTP client (query in 0.1s, no Claude session needed)
- Health check cached for 60 seconds
- Automatic lifecycle recording (plan/build/evaluate events)
- `auto_record` config toggle

**Phase 11: Custom Evaluators + External Integrations (done)**
- `harness evaluator list/use` for pluggable evaluator strategies
- Built-in strategies: default, playwright-mcp, curl
- Notification plugins: Slack, Telegram, email, webhook
- Notification events: on_eval_pass, on_eval_fail, on_eval_revise, on_schedule_complete
- SCL auto-records evaluator strategy and notification events

**Phase 12: Multi-Agent Orchestration (done)**
- `harness agent add/list/remove` for TOML-based agent definitions
- Agent roles: planner, builder, evaluator, custom
- `harness run --agents planner,builder,evaluator` for sequential multi-agent runs
- `harness run --workflow <name>` for TOML-defined workflow execution
- Workflow steps with optional prompt overrides
- SCL auto-records agent runs, individual steps, and outcomes
- 20 integration tests passing

**Phase 13: Parallel Agents + Iterative Loops + Workflow Validation (done)**
- `harness run --agents a,b --parallel` for concurrent agent execution
- `parallel: true` workflow steps for concurrent step batches
- `loop_until = "pass"` for iterative build-evaluate loops in workflows
- `harness workflow list/validate` for pre-run workflow validation
- SCL records parallel groups, loop iterations, and outcomes
- 24 integration tests passing

**Phase 14: Parallel Safety + TUI Visibility + Hook Support (done)**
- Artifact isolation for parallel agents (.harness/agents/<name>/) with auto-merge
- Thread-safe parallel hooks (before/after batch firing)
- Multi-agent TUI with parallel batch display, loop counters, and agent step info
- 25 integration tests

**Phase 15: Real-time Multi-Agent TUI Streaming + Full Artifact Overrides (done)**
- Real-time streaming for parallel agents via channel multiplexing
- Per-agent TUI output with color-coded `[agent-name]` prefixes
- Agent filter cycling (backtick key) and direct filter keys (4-7)
- Full `output_artifact` override support in parallel mode
- 26 integration tests

**Phase 16: Sequential Streaming + TUI Legend + Final Polish (done)**
- Sequential multi-agent steps use streaming backends (matches parallel)
- On-screen agent legend in status panel with color dots and key hints
- "Harness Multi-Agent" title header when in agent/workflow mode
- Iterative loops send real-time phase updates to TUI
- 26 integration tests

**Phase 17: SanctumAI Credential Vault Integration (done)**
- `harness vault init/status/add/list` commands
- Ed25519 agent identity with auto-generated signing key
- Vault-aware notification credential resolution (vault-first, config-fallback)
- Well-known credential paths for Slack, Telegram, email, webhook
- 28 integration tests

**Phase 18: Telegram Command Bridge (done)**
- `harness bridge telegram start/status/stop` for chat-based control
- Bot commands: `/run`, `/status`, `/agent list`, `/vault status`, `/help`
- Systemd-managed background service with auto-restart
- Vault-only credentials (bot token + chat ID from SanctumAI)
- SCL records every bridge command and response
- Long-polling (no webhooks/open ports needed)
- 30 integration tests

**Phase 19: Telegram Bridge Feedback Loop + Polish (done)**
- Workflow completion callback — `/run` sends results back on finish
- `/run --wait` mode with periodic progress updates (60s interval, 30m timeout)
- Vault policy authorization (`harness:bridge:telegram:<cmd>`) before all commands
- Robust Markdown escaping with automatic plain-text fallback
- Active workspace discovery for workflow execution
- 10 unit tests + 30 integration tests

**Phase 20: Telegram Bridge Final Polish + Configurable Timeouts & Rich Progress (done)**
- Configurable workflow timeout: global `[bridge].workflow_timeout_minutes` + per-workflow `timeout_minutes`
- Strict policy mode: `strict_policy_mode = true` denies commands when vault is unreachable
- Rich progress in `--wait` mode: per-agent status, feedback round counts, multi-agent output
- Agent completion summary in workflow result messages
- 15 unit tests + 30 integration tests

**Phase 21: Real-time Progress Protocol + Policy Robustness (done)**
- Progress log protocol: `.harness/progress.log` written by runner with timestamped agent events
- Live progress in Telegram: step starts, completions, verdicts, loop iterations, parallel batches
- `require_policy_endpoint` config flag: `_policy` vault endpoint is optional by default
- Improved policy error messages with hints for misconfiguration
- 15 unit tests + 31 integration tests

**Phase 22: Sub-second Progress Protocol + Raw Stdout Capture (done)**
- Unix domain socket progress protocol (`.harness/progress.sock`) for sub-second IPC
- `ProgressSender`/`ProgressListener` with `EVENT:`, `STDOUT:`, `DONE:` message format
- Non-TUI runner switches to streaming mode when progress socket is available
- Raw agent stdout lines forwarded with agent-name prefix
- Telegram bridge creates socket, passes via `HARNESS_PROGRESS_SOCK` env, reads live output
- Automatic fallback to file-based `progress.log` when socket unavailable
- 22 unit tests + 31 integration tests

**Phase 23: Final Telegram + Progress Protocol Polish (done)**
- Event-driven smart batching: send on significant events, rate-limited to 1 msg / 6s
- Multi-client socket listener with thread-per-connection
- progress.log written by listener only (removed dual writes from runner)
- Removed `append_progress`/`clear_progress_log` from artifacts module
- 24 unit tests + 31 integration tests

**Phase 24: Final Progress Protocol + Listener Shutdown Polish (done)**
- `ListenerHandle` with `Drop`-based shutdown via `AtomicBool` — no hard timeout
- Priority progress buffer: EVENT/DONE always kept, STDOUT evicted first
- Configurable `progress_buffer_size` (default: 50 lines)
- Client read timeout + shutdown check for clean thread exit
- 27 unit tests + 31 integration tests

**Phase 25: Agent Specialization + Cross-Project Learning (in progress)**
- Agent specialization metadata: `specializations`, `context_scopes`, `default_for`
- `@selector` routing for workflows and ad-hoc `--agents`
- Workflow `requires` validation for specialized steps
- Cross-project learning via Shared Context Layer

## License

MIT
