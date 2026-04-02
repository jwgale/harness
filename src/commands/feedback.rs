use crate::artifacts;

pub fn run() -> Result<(), String> {
    artifacts::ensure_harness_exists()?;

    let latest_num = artifacts::next_feedback_number() - 1;
    if latest_num == 0 {
        println!("No evaluator feedback yet. Run `harness evaluate` first.");
        return Ok(());
    }

    let path = format!("feedback/round-{latest_num:03}.md");
    let content = artifacts::read_artifact(&path)?;

    println!("--- Feedback from round {latest_num} ---\n");
    println!("{content}");

    Ok(())
}
