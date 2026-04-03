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

/// Record evaluation results (includes evaluator strategy if non-default).
pub fn record_eval_complete(project: &str, round: u32, verdict: &str, strategy: &str) {
    let strategy_note = if strategy.is_empty() || strategy == "default" {
        String::new()
    } else {
        format!(" [strategy: {strategy}]")
    };
    let content = format!(
        "Evaluation round {round} for {project}: {verdict}{strategy_note}"
    );
    auto_record("active_work", &content);
}

/// Record a notification event.
pub fn record_notification(plugin: &str, strategy: &str, event: &str, success: bool) {
    let status = if success { "sent" } else { "failed" };
    let content = format!(
        "Notification {status}: plugin={plugin}, strategy={strategy}, event={event}"
    );
    auto_record("insight", &content);
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
