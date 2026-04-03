# Harness

A CLI tool that orchestrates **planner -> builder -> evaluator** loops using subscription CLI tools (Claude Code, Codex). Zero API cost.

Inspired by [Anthropic's harness architecture](https://www.anthropic.com/engineering/harness-design-long-running-apps) for long-running application development with Opus 4.6.

## v0.2.0 Release Notes

Harness v0.2.0 is the first production-ready release, built across 8 development phases:

- **Core Orchestrator** — plan -> build -> evaluate -> revise loop with TUI, prompt overrides, streaming output, and verdict parsing
- **Local Install** — one-liner `curl` installer, XDG-compliant directory layout, `cargo install --path .`
- **Persistent Daemon** — systemd user service with `harness daemon start/stop/status/logs`
- **Plugin System** — TOML-based plugins with 6 lifecycle hooks, configurable timeouts, hot-reload
- **Workspace Management** — `harness workspace register/list/remove` with inotify file watching
- **Scheduled Tasks** — cron-style scheduling with deduplication, local timezone, execution history, manual triggers
- **Shared Context Layer** — built-in MCP integration gives every session access to long-term memory and decisions
- **Mock Backend** — `--backend mock` for instant testing without real Claude/Codex
- **Integration Tests** — 10+ tests running in under 1 second

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
```

## Prerequisites

- **Claude Code CLI** authenticated (Claude Max / Team subscription), OR
- **Codex CLI** authenticated (ChatGPT Pro subscription)

## Quick Start

```bash
# 1. Initialize a new project harness
harness init "Build a CLI todo app in Rust with SQLite storage"

# 2. Run the full automated loop (launches TUI)
harness run --backend claude

# Or run with human review pauses (plain text mode)
harness run --backend claude --pause-after-plan --pause-after-eval

# Or run individual phases
harness plan --backend claude
# Review/edit .harness/spec.md
harness build --backend claude
harness evaluate --backend claude
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

### `harness run` options

- `--backend claude|codex` — which CLI backend to use
- `--max-rounds N` — maximum evaluation/revision rounds (default: 3)
- `--pause-after-plan` — pause for human review after planning
- `--pause-after-eval` — pause for human review after each evaluation
- `--no-tui` — disable TUI, use plain text output

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
- `1`/`2`/`3` — switch view mode

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
  prompts/             # Custom prompt overrides (optional)
  feedback/
    round-001.md       # Per-round evaluator feedback
  runs/
    run-001.json       # Run metadata (timing, outcome)
```

## Directory Layout (XDG)

Global state follows XDG conventions:

| Path | Purpose |
|------|---------|
| `~/.config/harness/` | Global config, plugin manifests |
| `~/.config/harness/plugins/` | Plugin TOML files |
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

# Query shared context
harness context query "recent architectural decisions"

# Record a decision
harness context record decision "Chose SQLite over Postgres for local-first storage"
```

Configure in `~/.config/harness/config.toml`:
```toml
[shared_context]
enabled = true
url = "http://127.0.0.1:3100/mcp"
```

Set `enabled = false` to disable SCL integration. If the SCL server is unreachable, harness silently falls back to running without it.

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
  "backend": "claude",
  "model": "claude-opus-4-6",
  "project_name": "my-project",
  "max_eval_rounds": 3,
  "builder_timeout_seconds": 1800,
  "evaluator_timeout_seconds": 600,
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

**Phase 10: Custom Evaluators + External Integrations**
- Custom evaluator strategies (Playwright MCP, curl, etc.)
- Notification hooks (Slack, email, webhooks)

**Phase 11: Multi-Agent Orchestration**
- Parallel builder sessions
- Agent specialization (frontend, backend, testing)
- Cross-project learning via Shared Context Layer

## License

MIT
