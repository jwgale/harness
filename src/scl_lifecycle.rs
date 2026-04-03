//! Automatic SCL lifecycle recording.
//! Called after plan/build/evaluate to record key events.

use crate::global_config::GlobalConfig;
use crate::scl;

/// Record that planning completed.
pub fn record_plan_complete(project: &str) {
    auto_record("active_work", &format!("Plan completed for {project} — spec.md generated"));
}

/// Record that a build round completed.
pub fn record_build_complete(project: &str, round: u32) {
    auto_record("active_work", &format!("Build round {round} completed for {project}"));
}

/// Record evaluation results.
pub fn record_eval_complete(project: &str, round: u32, verdict: &str, scores_summary: &str) {
    let content = format!(
        "Evaluation round {round} for {project}: {verdict}. {scores_summary}"
    );
    auto_record("active_work", &content);
}

fn auto_record(entry_type: &str, content: &str) {
    let gc = GlobalConfig::load();
    let Some(scl_cfg) = gc.scl() else { return };
    if !scl_cfg.auto_record() {
        return;
    }
    if !scl::is_healthy(scl_cfg.url()) {
        return;
    }

    scl::auto_record(scl_cfg.url(), entry_type, content);
    scl::save_last_event(&format!("[{}] {}", chrono::Local::now().format("%H:%M"), truncate(content, 60)));
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}
