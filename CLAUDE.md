# CLAUDE.md — Harness V3 Build

## What This Is
A Rust CLI tool that orchestrates planner → builder → evaluator loops using subscription CLI tools (Claude Code, Codex). Zero API cost.

## Read First
1. `spec/HARNESS-V3-BUILD-TASK.md` — your build task with Definition of Done
2. `spec/HARNESS-V3-SPEC.md` — the full architecture spec

## Build Rules
- Rust: `clap` + `std::process::Command` + `std::fs` + `serde_json` + `chrono`
- Keep orchestration mostly synchronous; use small async bridges only where an SDK requires it
- Prefer functions and concrete types over abstraction-heavy designs
- `cargo clippy -- -D warnings` must pass clean
- Prompt templates embedded via `include_str!`

## Shared Context Layer (MCP)
This project is connected to the Shared Context Layer at `http://10.0.0.113:3100/mcp`.
Use SCL tools to record architectural decisions, key progress, and learnings.
Git hooks are installed to auto-log commits and pushes to SCL.

## Git
- Commit per feature with meaningful messages
- Conventional prefixes: feat:, fix:, refactor:, docs:, test:
