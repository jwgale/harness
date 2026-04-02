# Build Task: Harness TUI — Live Split-Pane Terminal Interface

## Objective

Add a real-time TUI (terminal user interface) to the harness so humans can monitor builds as they happen. The current CLI is a black box — you run `harness run` and see nothing until it's done. The TUI shows live progress with syntax-highlighted code output.

## Read First
- `spec/HARNESS-V3-SPEC.md` — the overall architecture
- `src/cli_backend.rs` — current process invocation (needs streaming)
- `src/commands/run.rs` — the main loop

## Layout

```
┌─ Harness Status ──────────────┬─ Live Output ─────────────────────────┐
│                                │                                       │
│  Project: RustDo               │  // src/main.rs                       │
│  Phase:   BUILD (round 1/3)    │  use clap::{Parser, Subcommand};     │
│  Backend: claude               │  use serde::{Serialize, Deserialize}; │
│  Elapsed: 2m 34s               │                                       │
│                                │  #[derive(Parser)]                    │
│  ─── Spec Features ───         │  #[command(name = "rustdo")]          │
│  ✅ Feature 1: Add Tasks       │  struct Cli {                         │
│  ✅ Feature 2: List Tasks      │      #[command(subcommand)]           │
│  🔨 Feature 3: Complete Tasks  │      command: Option<Commands>,       │
│  ⬜ Feature 4: Delete Tasks    │  }                                    │
│  ⬜ Feature 5: Edit Tasks      │                                       │
│                                │                                       │
│  ─── Last Evaluation ───      │                                       │
│  functionality: 8/10           │                                       │
│  completeness:  8/10           │                                       │
│  code_quality:  6/10           │                                       │
│  design_quality: 7/10          │                                       │
│  robustness:    7/10           │                                       │
│  VERDICT: PASS                 │                                       │
│                                │                                       │
└────────────────────────────────┴───────────────────────────────────────┘
```

## Requirements

### 1. Streaming stdout from CLI backends

The current `cli_backend.rs` uses `wait_with_output()` which blocks until the process finishes. Change to incremental line-by-line reads:

- Spawn the process
- Read stdout line-by-line in a background thread
- Push lines to the TUI for live rendering
- Still capture the full output for artifact writing when done

This applies to planner, builder, AND evaluator — all three should stream.

### 2. Left pane — Status panel

Shows at all times:
- Project name (from config.json)
- Current phase: PLAN / BUILD / EVALUATE with round number
- Backend name (claude/codex)
- Elapsed time (updates every second)
- Feature checklist parsed from spec.md (detect "Feature N:" or "### Feature" patterns)
  - ⬜ not started
  - 🔨 in progress (detect from builder output — file writes, git commits mentioning the feature)
  - ✅ completed
- Last evaluation scores (after first evaluate pass)
- Current verdict

### 3. Right pane — Live output

- Scrolling view of the current agent's stdout
- Syntax highlighting using `syntect`:
  - Detect code blocks (```rust, ```python, etc) and highlight accordingly
  - Highlight file paths, error messages, git output
  - Default to markdown highlighting for non-code output
- Auto-scroll to bottom (follow mode)
- User can scroll up to review (pause auto-scroll), press End or 'f' to resume following

### 4. Keyboard controls

- `q` — quit (kill running process and exit)
- `f` — toggle follow mode (auto-scroll)
- `↑/↓` or `j/k` — scroll output pane
- `PgUp/PgDn` — page scroll
- `Tab` — toggle between full-width output and split view
- `1/2/3` — switch view: status+output (default), output only, status only

### 5. Integration with existing commands

- `harness run` — launches TUI by default
- `harness run --no-tui` — falls back to current plain text output (keep backward compat)
- `harness plan/build/evaluate` — can also use TUI with `--tui` flag
- The TUI is an overlay on the existing logic — don't rewrite the run loop, wrap it

## Dependencies to Add

```toml
ratatui = "0.29"
crossterm = "0.28"
syntect = "5"
```

## Implementation Notes

### Keep it simple
- No async/tokio — use `std::thread` for the background stdout reader
- Channel-based: stdout reader thread sends lines via `std::sync::mpsc` to the TUI render loop
- TUI render loop runs on the main thread with crossterm event polling
- Process management stays in `cli_backend.rs` — just add a streaming variant

### Suggested new files
- `src/tui/mod.rs` — TUI app state and render loop
- `src/tui/status_panel.rs` — left pane rendering
- `src/tui/output_panel.rs` — right pane with syntax highlighting
- `src/tui/spec_parser.rs` — extract feature list from spec.md
- `src/cli_backend.rs` — add `run_streaming()` variants that return a channel

### Don't break existing functionality
- All current commands must still work without TUI
- Plain text fallback via `--no-tui`
- Artifacts are still written the same way
- The run loop logic doesn't change — TUI is a display layer

## Definition of Done

1. `harness run --backend claude` launches the TUI
2. Left pane shows project, phase, elapsed time, and feature checklist
3. Right pane streams live output with syntax highlighting
4. Keyboard controls work (quit, scroll, follow toggle)
5. After evaluator runs, scores appear in left pane
6. `--no-tui` flag preserves current plain text behavior
7. `cargo clippy -- -D warnings` passes clean
8. Compiles and installs on Linux

## What NOT to Build
- No mouse support (keyboard only)
- No config file for TUI colors/layout (hardcode sensible defaults)
- No persistent log viewer (that's what .harness/ artifacts are for)
- No network features
