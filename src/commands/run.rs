use crate::artifacts;
use crate::commands::{build, evaluate, plan};
use crate::config::Config;
use crate::commands::evaluate::Verdict;
use std::io::{self, Write};

pub fn run(
    backend_override: Option<&str>,
    max_rounds: Option<u32>,
    pause_after_plan: bool,
    pause_after_eval: bool,
    no_tui: bool,
) -> Result<(), String> {
    // If TUI is enabled (default), delegate to the TUI module
    if !no_tui && !pause_after_plan && !pause_after_eval {
        return crate::tui::run_with_tui(backend_override, max_rounds);
    }
    run_plain(backend_override, max_rounds, pause_after_plan, pause_after_eval)
}

fn run_plain(
    backend_override: Option<&str>,
    max_rounds: Option<u32>,
    pause_after_plan: bool,
    pause_after_eval: bool,
) -> Result<(), String> {
    artifacts::ensure_harness_exists()?;
    let config = Config::load(&artifacts::harness_dir())?;
    let max = max_rounds.unwrap_or(config.max_eval_rounds);

    // Phase 1: Plan
    plan::run(backend_override)?;

    if pause_after_plan {
        println!("\n--- Plan complete. Review .harness/spec.md ---");
        println!("Press Enter to continue to build, or Ctrl+C to abort.");
        wait_for_enter();
    }

    // Phase 2+3: Build → Evaluate loop
    for round in 1..=max {
        println!("\n=== Round {round}/{max} ===\n");

        // Save run metadata
        save_run_metadata(round, backend_override.unwrap_or(&config.backend))?;

        // Build
        build::run(backend_override)?;

        // Evaluate
        let verdict = evaluate::run(backend_override)?;

        // Update run metadata with outcome
        update_run_outcome(round, &verdict)?;

        match verdict {
            Verdict::Pass => {
                println!("\n=== BUILD PASSED on round {round} ===");
                return Ok(());
            }
            Verdict::Revise => {
                if round == max {
                    println!("\n=== Max rounds ({max}) exhausted. Last verdict: REVISE ===");
                    println!("Check .harness/evaluation.md for details.");
                    return Err("Max revision rounds exhausted".to_string());
                }
                println!("\nVerdict: REVISE — looping back to builder with feedback.");

                if pause_after_eval {
                    println!("Press Enter to continue to next round, or Ctrl+C to abort.");
                    wait_for_enter();
                }
            }
            Verdict::Fail => {
                println!("\n=== BUILD FAILED on round {round} ===");
                println!("Check .harness/evaluation.md for details.");
                return Err("Evaluator returned FAIL verdict".to_string());
            }
        }
    }

    Ok(())
}

fn wait_for_enter() {
    let mut input = String::new();
    let _ = io::stdout().flush();
    let _ = io::stdin().read_line(&mut input);
}

fn save_run_metadata(round: u32, backend: &str) -> Result<(), String> {
    let run_num = artifacts::next_run_number();
    let metadata = serde_json::json!({
        "id": run_num,
        "round": round,
        "phase": "build+evaluate",
        "backend": backend,
        "started_at": chrono::Utc::now().to_rfc3339(),
        "ended_at": null,
        "outcome": null,
    });
    let json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| format!("Failed to serialize run metadata: {e}"))?;
    artifacts::write_artifact(&format!("runs/run-{run_num:03}.json"), &json)
}

fn update_run_outcome(round: u32, verdict: &Verdict) -> Result<(), String> {
    // Find the run file for this round and update it
    let run_num = if round == 1 { 1 } else { round };
    let path = format!("runs/run-{run_num:03}.json");
    if let Ok(content) = artifacts::read_artifact(&path)
        && let Ok(mut meta) = serde_json::from_str::<serde_json::Value>(&content)
    {
        meta["ended_at"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
        meta["outcome"] = serde_json::json!(format!("{verdict:?}"));
        let json = serde_json::to_string_pretty(&meta)
            .map_err(|e| format!("Failed to serialize: {e}"))?;
        artifacts::write_artifact(&path, &json)?;
    }
    Ok(())
}
