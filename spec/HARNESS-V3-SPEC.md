# Harness V3 Specification

**Purpose:** A minimal, practical harness for long-running application development. Inspired by Anthropic's actual V3 architecture (Opus 4.6), not our earlier over-engineered spec.

**Key references:**
- https://www.anthropic.com/engineering/harness-design-long-running-apps
- https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents
- `research/agent-harness/HARNESS-PART-2-SPEC.md` (predecessor — preserved for reference)

**Core stance:** Start with what Anthropic found necessary for Opus 4.6 and current-gen models. Add complexity only when we hit real walls. Every component must justify its existence by solving a problem we actually encounter.

---

## 1. What Anthropic Actually Shipped (V3)

Their final harness for Opus 4.6:

1. **Planner** — takes a 1-4 sentence prompt, expands it into a full product spec with features, design language, and technical direction. Intentionally high-level. Does NOT specify implementation details (those cascade errors downstream).

2. **Builder (Generator)** — one continuous session. No sprint decomposition. Works through the spec feature by feature. Has git, file system, and build tools. Self-evaluates at the end of each feature before handing off to QA. Opus 4.6 sustains coherent work for 2+ hours without context resets.

3. **Evaluator (QA)** — runs after the build pass. Uses Playwright MCP to actually interact with the running application. Grades against criteria with hard thresholds. If any criterion fails, loops back to builder with specific feedback. NOT a rubber stamp.

4. **Communication** — via files. One agent writes, another reads. No message bus, no RPC, no shared memory.

5. **Simplification** — sprints removed, contract negotiation removed, context resets removed. All were necessary for Sonnet 4.5 / Opus 4.5 but unnecessary for Opus 4.6.

**Their results:** Browser DAW built in ~4 hours, $125 total. Core music production features working end-to-end.

---

## 2. Our Architecture

### Principles

**P0. Zero API cost.** All LLM interaction routes through subscription CLIs (Claude Code, Codex). No direct API calls. No per-token billing.

**P1. Start simple, add complexity when you hit the wall.** Every harness component encodes an assumption about what the model can't do alone. Test those assumptions before building machinery around them.

**P2. Files are the communication layer.** Agents read and write files in `.harness/`. No orchestration protocol needed.

**P3. The harness is a build pipeline, not a chat interface.** Brainstorming and ideation happen outside the harness (in normal conversation). The harness activates when you have a clear goal to execute.

**P4. Human checkpoints are mandatory.** The harness pauses for human review after planning and after evaluation. No auto-advancing through the full loop.

**P5. Creation and judgment must be separated.** The builder does not evaluate its own work. A separate evaluator session does.

### What lives OUTSIDE the harness

- Brainstorming and ideation → normal Claude Code / Codex / Clawvis conversation
- Research and exploration → manual or agent-assisted, results fed into harness as input
- Architecture decisions → human makes them, harness executes them
- Tool/framework selection → decided before harness runs

### What lives INSIDE the harness

- Goal → spec expansion (planner)
- Spec → working code (builder)
- Working code → quality assessment (evaluator)
- Assessment → feedback loop or completion

---

## 3. Artifact Directory

```
project-root/
  .harness/
    spec.md              # Planner output: full product spec
    status.md            # Current state: what's done, what's next, blockers
    evaluation.md        # Latest evaluator assessment
    handoff.md           # Brief for next session if context resets needed
    feedback/             
      round-001.md       # Evaluator feedback from round 1
      round-002.md       # Evaluator feedback from round 2
      ...
    runs/
      run-001.json       # Metadata: who ran, when, duration, outcome
      run-002.json
      ...
```

### Artifact rules

- **spec.md** — written by planner, read by builder and evaluator. Human reviews and edits before builder starts.
- **status.md** — updated by builder at end of each build pass. Human-readable summary.
- **evaluation.md** — written by evaluator. Structured: criterion scores + verdict + specific failures + recommendations.
- **handoff.md** — written by orchestrator when a context reset is needed. Contains everything the next session needs to pick up cleanly.
- **feedback/round-NNN.md** — evaluator feedback per round, preserved for history. Builder reads the latest one.

---

## 4. Roles

### 4.1 Planner

**Input:** 1-4 sentence goal from human
**Output:** `spec.md` — full product specification

**Prompt posture:**
- Be ambitious about scope
- Focus on product context and high-level technical design
- Do NOT specify granular implementation details (errors cascade)
- Find opportunities for AI-powered features where they add genuine value
- Include a design language / visual direction section
- Structure as features with clear deliverables

**Tool access:** Read-only. Web search for reference. No file mutation.

**Backend:** `claude --print --dangerously-skip-permissions --model claude-opus-4-6 -p` or `codex exec -q`

**Human checkpoint:** After planner outputs spec.md, human reviews and edits before proceeding to builder.

### 4.2 Builder (Generator)

**Input:** `spec.md` + `feedback/round-NNN.md` (if revision loop) + project working directory
**Output:** Working code + updated `status.md`

**Prompt posture:**
- Work through spec feature by feature
- Use git commits per feature
- Self-evaluate briefly after each feature (but this is NOT the formal evaluation)
- Report blockers honestly in status.md
- If something in the spec seems wrong or impossible, note it in status.md rather than silently diverging

**Tool access:** Full — read, write, execute, build, test, git. This is the agent doing real work.

**Backend:** Interactive Claude Code session (not one-shot `--print`). Builder needs full tool access and should run as a normal `claude` session with the project directory as working dir, OR `codex` interactive session.

**Important:** The builder runs as a real Claude Code / Codex session, not a one-shot prompt. This is a critical difference from our V2 spec. The subscription covers multi-hour interactive sessions. Use them.

### 4.3 Evaluator (QA)

**Input:** Running application + `spec.md` + `status.md`
**Output:** `evaluation.md` + `feedback/round-NNN.md`

**Prompt posture:**
- Skeptical by default
- Actually use the application (Playwright MCP, curl, browser, whatever fits)
- Grade each criterion with a score and specific evidence
- Hard thresholds: if any criterion fails, the round fails
- Be specific about what's broken and where in the code
- Do NOT talk yourself into approving mediocre work

**Evaluation criteria (adapted from Anthropic):**
1. **Functionality** — does it work? Can a user complete core tasks?
2. **Completeness** — does it cover the spec? What's missing?
3. **Code quality** — is it maintainable, tested, not held together with tape?
4. **Design quality** — does the UI feel intentional, not default/generic?
5. **Robustness** — edge cases, error handling, does it break on unusual input?

**Tool access:** Read + inspect + execute tests + interact with running app. No broad code mutation.

**Backend:** `claude --print` (one-shot is fine for evaluation — it's a judgment call, not a build session). Send spec + status + ability to interact with the project.

**Human checkpoint:** After evaluator outputs, human decides: accept, send back for revision, or abandon.

---

## 5. Orchestration

### The orchestrator is intentionally thin

The orchestrator's job:
1. Create `.harness/` directory structure
2. Invoke planner → write spec.md → **pause for human review**
3. Invoke builder (interactive session) → builder works until done → writes status.md
4. Invoke evaluator → write evaluation.md + feedback → **pause for human review**
5. If human says revise: invoke builder again with latest feedback
6. If human says accept: mark run complete
7. If context reset needed: generate handoff.md, start fresh builder session

### Implementation: Rust CLI, simple and installable

Build as a Rust binary (`harness`) using `clap` + `std::process::Command` + `std::fs`. No async, no traits, no generics. Just functions that read files, build strings, invoke CLIs, and write files. Installable via `cargo install --path .`.

Prompt templates are embedded via `include_str!` so the binary is self-contained. Users can override prompts via `.harness/prompts/` directory.

The `harness run` command automates the full loop: plan → build → evaluate → (revise or accept). With `--pause-after-plan` and `--pause-after-eval` flags for human checkpoints when desired.

### Command interface (target)

```bash
# Initialize a new project harness
harness init "Build a REST API for task management with AI-powered prioritization"

# Run the planner (outputs spec.md, then pauses)
harness plan [--backend claude|codex]

# Review spec before proceeding
# (human edits .harness/spec.md if needed)

# Run the builder (starts interactive session)
harness build [--backend claude|codex]

# Run the evaluator (outputs evaluation.md)
harness evaluate [--backend claude|codex]

# Check current state
harness status

# Generate handoff brief for context reset
harness reset

# Full loop with human checkpoints
harness run [--backend claude|codex]
# Equivalent to: plan → [pause] → build → evaluate → [pause] → (revise or accept)
```

---

## 6. Subscription CLI Integration

### Claude Code (Claude Max / Team)

**Planner (one-shot):**
```bash
cat .harness/planner-prompt.md | claude --print --dangerously-skip-permissions --model claude-opus-4-6 -p > .harness/spec.md
```

**Builder (interactive session):**
```bash
cd $PROJECT_DIR
claude --model claude-opus-4-6 --dangerously-skip-permissions
# Builder works interactively with full tool access
# Reads .harness/spec.md and .harness/feedback/ for context
# Session runs until builder reports completion or human intervenes
```

**Evaluator (one-shot):**
```bash
cat .harness/evaluator-prompt.md | claude --print --dangerously-skip-permissions --model claude-opus-4-6 -p > .harness/evaluation.md
```

### Codex (ChatGPT Pro)

**Planner:**
```bash
cat .harness/planner-prompt.md | codex exec -q > .harness/spec.md
```

**Builder:**
```bash
cd $PROJECT_DIR
codex  # interactive session
```

**Evaluator:**
```bash
cat .harness/evaluator-prompt.md | codex exec -q > .harness/evaluation.md
```

### Backend selection

The orchestrator picks the backend based on `--backend` flag or a config file (`.harness/config.json`):
```json
{
  "backend": "claude",
  "model": "claude-opus-4-6",
  "project_name": "my-project",
  "max_eval_rounds": 3
}
```

---

## 7. What We Deferred (and Why)

| Component | V2 Spec Had It | V3 Status | Reason |
|-----------|----------------|-----------|--------|
| Rust crate with traits/generics | Yes | Deferred | Start with shell, graduate when flow is proven |
| Run manager | Yes | Simplified to shell | Over-engineering for current model capabilities |
| Role dispatcher | Yes | Removed | CLI invocation is the dispatcher |
| Policy engine | Yes | Removed | Model capabilities make per-tool policies unnecessary — builder gets full access, evaluator prompt constrains behavior |
| Tool registry | Yes | Removed | Claude Code / Codex have their own tool systems |
| Context loader | Yes | Replaced by file reads | Artifacts are files. Read them. |
| Sprint decomposition | Yes | Removed | Opus 4.6 doesn't need it (Anthropic confirmed) |
| Context resets | Optional | Optional | Opus 4.6 handles long sessions via compaction; keep handoff.md as escape hatch |
| Conversation/brainstorm mode | Considered | Out of scope | Happens outside harness in normal sessions |
| Multi-agent swarms | Deferred in V2 | Still deferred | Start with 3 roles, expand only if needed |

---

## 8. What to Build First

### Phase 1: Rust CLI + prompts (2-3 days)

1. Scaffold the Rust crate with `clap` commands
2. Implement `init`, `plan`, `build`, `evaluate`, `run`, `status`, `reset`, `feedback`
3. Write strong prompts for each role (this is the real work):
   - `prompts/planner.md` — role instructions + output format
   - `prompts/builder.md` — role instructions + how to read spec + how to report status
   - `prompts/evaluator.md` — role instructions + criteria + scoring format + skepticism calibration
4. Wire up Claude Code + Codex CLI invocation
5. Implement the `run` loop with verdict parsing and feedback cycling
6. `cargo install --path .` should produce a working binary

### Phase 2: Validate on a real build (1-2 days)

Pick a concrete first project and run the harness end-to-end:
- **Candidate A:** The harness building itself (meta but useful)
- **Candidate B:** A small but real tool — maybe a CLI for managing the SCL, or a dashboard for harness run history
- **Candidate C:** Something for JJ (game-related?)

The first real run will reveal what's missing. That's the signal for what to add.

### Phase 3: Iterate on prompts (ongoing)

Anthropic says this is where the real work lives. Read evaluator logs, find where judgment diverges from yours, tune prompts. This is a human-in-the-loop activity, not a build task.

### Phase 4: Polish and harden

- Better error messages and recovery
- TUI for status/review (optional)
- Prompt override system (`.harness/prompts/` overrides embedded defaults)
- Support for different evaluator strategies (one-shot vs interactive with Playwright MCP)
- Shared Context Layer integration for cross-project learning

---

## 9. Success Criteria

The harness is successful when:

1. You give it a 1-4 sentence goal
2. The planner produces a spec you'd actually approve (with minor edits)
3. The builder produces working code that covers the spec
4. The evaluator catches real issues the builder missed
5. After one revision loop, the output is shippable
6. Total cost: $0 (subscription CLIs)
7. Total time: comparable to Anthropic's results (2-6 hours for a full app)
8. You'd reach for the harness over raw Claude Code for projects above a certain complexity threshold

---

## 10. What This Is NOT

- Not a chat interface (brainstorm with Clawvis or raw Claude Code)
- Not a platform (no UI, no marketplace, no plugins)
- Not tied to FlowCanvas (standalone, clean break)
- Not a framework (it's a workflow tool for Jason's builds)
- Not permanent architecture (will simplify further as models improve, will add complexity when hitting real walls)

---

## Appendix A: Anthropic's Key Quote

> "Every component in a harness encodes an assumption about what the model can't do on its own, and those assumptions are worth stress testing, both because they may be incorrect, and because they can quickly go stale as models improve."

> "Find the simplest solution possible, and only increase complexity when needed."

## Appendix B: Relationship to V2 Spec

The V2 spec (`HARNESS-PART-2-SPEC.md`) and its scaffold (`harness-v2-scaffold/`) are preserved as reference. The core ideas — bounded runs, artifact continuity, separated evaluation — carry forward. The implementation approach changes from "Rust crate first" to "shell script first, prove the flow, then harden."

The V2 scaffold code may be useful when graduating to Rust in Phase 4. The types, artifact store, and evaluator parsing are reusable foundations.
