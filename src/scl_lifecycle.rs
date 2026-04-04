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

/// Record start of a multi-agent run.
pub fn record_agent_run_start(project: &str, agents: &[&str]) {
    let names = agents.join(", ");
    auto_record("active_work", &format!(
        "Multi-agent run started for {project} — agents: [{names}]"
    ));
}

/// Record completion of an individual agent step.
pub fn record_agent_step(project: &str, agent: &str, role: &str, status: &str) {
    auto_record("active_work", &format!(
        "Agent '{agent}' ({role}) {status} for {project}"
    ));
}

/// Record end of a multi-agent run.
pub fn record_agent_run_end(project: &str, agents: &[&str], outcome: &str) {
    let names = agents.join(", ");
    auto_record("active_work", &format!(
        "Multi-agent run {outcome} for {project} — agents: [{names}]"
    ));
}

/// Record start of a parallel step group.
pub fn record_parallel_start(project: &str, agents: &[&str]) {
    let names = agents.join(", ");
    auto_record("active_work", &format!(
        "Parallel execution started for {project}: [{names}]"
    ));
}

/// Record end of a parallel step group.
pub fn record_parallel_end(project: &str, agents: &[&str]) {
    let names = agents.join(", ");
    auto_record("active_work", &format!(
        "Parallel execution completed for {project}: [{names}]"
    ));
}

/// Record a loop iteration.
pub fn record_loop_iteration(project: &str, round: u32, max: u32) {
    auto_record("active_work", &format!(
        "Build-evaluate loop iteration {round}/{max} for {project}"
    ));
}

/// Record a bridge lifecycle event (start/stop).
pub fn record_bridge_event(bridge: &str, event: &str, detail: &str) {
    auto_record("active_work", &format!(
        "Bridge {bridge} {event}: {detail}"
    ));
}

/// Record a bridge command received.
pub fn record_bridge_command(bridge: &str, user: &str, cmd: &str, args: &str) {
    let args_note = if args.is_empty() {
        String::new()
    } else {
        format!(" {args}")
    };
    auto_record("insight", &format!(
        "Bridge {bridge} command from @{user}: {cmd}{args_note}"
    ));
}

/// Record a bridge command response.
pub fn record_bridge_response(bridge: &str, cmd: &str, response: &str) {
    auto_record("insight", &format!(
        "Bridge {bridge} response to {cmd}: {}",
        truncate(response, 100)
    ));
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
    if s.len() <= max {
        s
    } else {
        // Find a valid char boundary at or before max
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}
