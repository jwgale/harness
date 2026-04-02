# Build Task: Harness V3 — Installable CLI + Prompts

**Target machine:** 5090 (Ubuntu 24.04)
**Builder:** Claude Code or Codex (interactive session)
**Working directory:** Create `~/projects/harness` on the 5090
**Duration estimate:** 4-6 hours
**Prerequisites:** Claude Code CLI authenticated, Codex CLI authenticated

---

## Read First

1. `HARNESS-V3-SPEC.md` (this directory) — the full specification
2. The Anthropic article: https://www.anthropic.com/engineering/harness-design-long-running-apps

## Objective

Build a working, installable harness CLI that takes a 1-4 sentence build goal and orchestrates planner → builder → evaluator in an automated loop using subscription CLI tools. The end result must be a single binary or script that a user can install and run on Linux with no manual glue.

## Definition of Done

A user on a fresh Ubuntu machine (with Claude Code or Codex CLI authenticated) can:
1. Install the harness (`cargo install` or `cp harness /usr/local/bin/`)
2. Run `harness init "Build a CLI todo app in Rust"` in an empty directory
3. Run `harness run --backend claude` and walk away
4. Come back to find: a generated spec, built code, evaluator feedback, and either a PASS verdict or multiple revision rounds attempted
5. Inspect `.harness/` to see the full audit trail

If it doesn't do those 5 things end-to-end without manual intervention (except optional human review pauses), it's not done.

## Deliverables

### 1. `harness` — Rust CLI binary (installable via `cargo install --path .`)

**Why Rust, not shell:** The orchestration needs to manage subprocesses, parse structured output, handle timeouts, manage the feedback loop, and be installable as a single binary. Shell gets messy fast for this. Keep the Rust simple — no traits, no generics, no async. Just `clap` + `std::process::Command` + `std::fs`.

Commands:
```
harness init <goal>                       # Create .harness/ dir, write goal, generate config
harness plan [--backend claude|codex]     # Run planner, output spec.md
harness build [--backend claude|codex]    # Launch builder session (automated, non-interactive)
harness evaluate [--backend claude|codex] # Run evaluator, output evaluation.md
harness run [--backend claude|codex] [--max-rounds 3] [--pause-after-plan] [--pause-after-eval]
                                          # Full automated loop: plan → build → evaluate → revise loop
harness status                            # Print current harness state from artifacts
harness reset                             # Generate handoff.md for context reset
harness feedback                          # Show latest evaluator feedback
```

### The `run` command (the main event)

`harness run` executes the full loop:

```
1. PLAN: Invoke planner → write spec.md
   - If --pause-after-plan: print spec, wait for user confirmation (stdin prompt)
   - Otherwise: continue automatically

2. BUILD: Invoke builder → builder reads spec, writes code, updates status.md
   - Builder runs as one-shot Claude Code (--print mode with full project context)
   - NOT interactive — the harness assembles the full prompt and captures output
   - Builder prompt includes: spec.md + project file listing + any feedback from prior rounds

3. EVALUATE: Invoke evaluator → reads spec + status + project files → writes evaluation.md
   - Evaluator runs as one-shot
   - Parses verdict from evaluator output: PASS, REVISE, or FAIL

4. DECISION:
   - If PASS: done. Print summary. Exit 0.
   - If REVISE and rounds < max_rounds: loop back to BUILD with feedback
   - If FAIL or rounds exhausted: print summary with what went wrong. Exit 1.
   - If --pause-after-eval: prompt user before looping

5. Each round writes: runs/run-NNN.json, feedback/round-NNN.md
```

### Builder execution model

Important design decision: the builder runs via **Claude Code `--print` with `--dangerously-skip-permissions`**. In this mode, Claude Code can still read/write files and execute commands in the working directory — it's not just a text-in/text-out pipe. It's a full agent session that happens to run non-interactively.

```bash
# Builder invocation — Claude Code does the actual file I/O
cat "$BUILDER_PROMPT_FILE" | claude --print --dangerously-skip-permissions --model claude-opus-4-6 -p
# Claude reads/writes project files directly in the working directory
# No need for the harness to parse output and apply patches
```

For Codex:
```bash
cat "$BUILDER_PROMPT_FILE" | codex exec -q --full-auto
# Codex in full-auto mode handles file operations directly
```

The key insight: the subscription CLIs are already full agent harnesses themselves. Our harness wraps them with structure (spec → build → evaluate → feedback loop), not with file I/O plumbing.

**Timeout handling:** Builder invocations should have a configurable timeout (default: 30 minutes). If the builder hasn't completed within the timeout, kill the process, capture whatever status.md was written, and proceed to evaluation of partial work.

### 2. Prompt templates

Located in `prompts/` directory:

#### `prompts/planner.md`
Role instructions for the planner. Must include:
- Expand the goal into an ambitious but achievable product spec
- Focus on product context and high-level technical design
- Do NOT specify granular implementation details
- Include a features list with clear deliverables
- Include a design direction / visual language section
- Structure output as a clean markdown spec document

#### `prompts/builder.md`
Initial prompt injected into the builder's interactive session. Must include:
- Read .harness/spec.md for the product specification
- Read .harness/feedback/ for any evaluator feedback from previous rounds
- Work through the spec feature by feature
- Use git commits per feature (meaningful commit messages)
- Update .harness/status.md as you complete features
- If something in the spec seems wrong, note it in status.md rather than silently diverging
- When done, update status.md with final state and exit

#### `prompts/evaluator.md`
Role instructions for the evaluator. Must include:
- You are a skeptical QA engineer evaluating a build
- Read .harness/spec.md for what was supposed to be built
- Read .harness/status.md for what the builder claims to have done
- Actually inspect the code, run tests, and if possible interact with the running application
- Grade each criterion (1-10) with specific evidence:
  1. Functionality — does it work? Can a user complete core tasks?
  2. Completeness — does it cover the spec? What features are missing?
  3. Code quality — maintainable, tested, reasonable architecture?
  4. Design quality — intentional UI, not defaults/generic? (if applicable)
  5. Robustness — edge cases, error handling, failure modes?
- Hard threshold: any criterion below 5 = round fails
- Overall verdict: PASS, REVISE (with specific fixes needed), or FAIL
- List specific failures with file paths and line numbers where possible
- List concrete recommendations for the next build round
- Output format must be parseable (structured markdown with scores)

### 3. `.harness/` directory structure

Created by `harness init`:
```
.harness/
  config.json          # backend, model, project name, max rounds
  goal.md              # Original goal text
  spec.md              # Planner output (after plan phase)
  status.md            # Builder-maintained status
  evaluation.md        # Latest evaluator output
  handoff.md           # Generated on reset
  feedback/
    round-001.md       # Per-round evaluator feedback
  runs/
    run-001.json       # {id, phase, backend, started_at, ended_at, outcome}
```

### 4. README.md

Installation and usage instructions. Including:
- Prerequisites (Claude Code CLI, Codex CLI, authentication)
- Quick start example
- How to customize prompts
- How the feedback loop works

---

## Implementation Notes

### Rust style

Keep it dead simple:
- `clap` for CLI parsing
- `std::process::Command` for subprocess invocation
- `std::fs` for file I/O
- `serde_json` for config/run metadata
- `chrono` for timestamps
- NO async, NO traits, NO generics, NO tokio
- Functions, not abstractions. `fn run_planner(config: &Config) -> Result<()>`

### Project structure

```
harness/
  Cargo.toml
  src/
    main.rs           # CLI entry point (clap)
    commands/
      init.rs         # harness init
      plan.rs         # harness plan
      build.rs        # harness build
      evaluate.rs     # harness evaluate
      run.rs          # harness run (full loop)
      status.rs       # harness status
      reset.rs        # harness reset
      feedback.rs     # harness feedback
    cli_backend.rs    # Claude/Codex invocation helpers
    artifacts.rs      # .harness/ file management
    prompts.rs        # Prompt assembly from templates + artifacts
    config.rs         # Config types + defaults
  prompts/
    planner.md
    builder.md
    evaluator.md
  README.md
```

### Prompt templates are embedded at compile time

Use `include_str!` to embed prompt templates into the binary:
```rust
const PLANNER_PROMPT: &str = include_str!("../prompts/planner.md");
const BUILDER_PROMPT: &str = include_str!("../prompts/builder.md");
const EVALUATOR_PROMPT: &str = include_str!("../prompts/evaluator.md");
```

This means the binary is self-contained — no external files needed after install. User can override with custom prompts via `.harness/prompts/` if desired.

### Evaluator verdict parsing

The evaluator prompt instructs output in a parseable format:
```
VERDICT: PASS|REVISE|FAIL
SCORES:
  functionality: 8/10
  completeness: 7/10
  code_quality: 6/10
  design_quality: 7/10
  robustness: 5/10
FAILURES:
  - [file.rs:42] Error handling missing for network timeout
  - [main.rs:15] Hardcoded config path
RECOMMENDATIONS:
  - Add timeout handling to all HTTP calls
  - Move config to .harness/config.json
```

The harness parses the VERDICT line to decide loop behavior. If parsing fails, treat as REVISE (conservative).

### Config

`.harness/config.json`:
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

---

## Testing

### Smoke test
1. `harness init "Build a simple CLI todo app in Rust"`
2. `harness plan --backend claude`
3. Review spec.md — is it reasonable?
4. `harness build --backend claude` (or start a Claude Code session manually)
5. Build the todo app
6. `harness evaluate --backend claude`
7. Does the evaluator catch real issues?
8. Revision loop if needed

### What to report when done

- Does `harness init/plan/evaluate/status` work end-to-end?
- How does the planner spec quality look?
- How does the evaluator feedback quality look?
- What prompt iterations were needed?
- What's still rough or missing?
- Total time and cost (should be $0 for subscription)

---

## What NOT to Build

- No web UI
- No database
- No plugin system
- No multi-agent swarm
- No context reset automation (just generate handoff.md, human decides)
- No Rust traits/generics/async unless it genuinely simplifies the code
- No integration with FlowCanvas, OpenClaw, or any other system
