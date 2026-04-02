mod artifacts;
mod cli_backend;
mod commands;
mod config;
mod prompts;
mod tui;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "harness", about = "Orchestrate planner → builder → evaluator loops using subscription CLI tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create .harness/ directory, write goal, generate config
    Init {
        /// The build goal (1-4 sentences)
        goal: String,
    },
    /// Run planner to generate spec.md from the goal
    Plan {
        /// Backend to use: claude or codex
        #[arg(long)]
        backend: Option<String>,
    },
    /// Run builder to implement the spec
    Build {
        /// Backend to use: claude or codex
        #[arg(long)]
        backend: Option<String>,
    },
    /// Run evaluator to assess the build
    Evaluate {
        /// Backend to use: claude or codex
        #[arg(long)]
        backend: Option<String>,
    },
    /// Full automated loop: plan → build → evaluate → revise
    Run {
        /// Backend to use: claude or codex
        #[arg(long)]
        backend: Option<String>,
        /// Maximum evaluation/revision rounds
        #[arg(long)]
        max_rounds: Option<u32>,
        /// Pause for human review after planning
        #[arg(long)]
        pause_after_plan: bool,
        /// Pause for human review after each evaluation
        #[arg(long)]
        pause_after_eval: bool,
        /// Disable TUI and use plain text output
        #[arg(long)]
        no_tui: bool,
    },
    /// Print current harness state from artifacts
    Status,
    /// Generate handoff.md for context reset
    Reset,
    /// Show latest evaluator feedback
    Feedback,
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Init { goal } => commands::init::run(&goal),
        Commands::Plan { backend } => commands::plan::run(backend.as_deref()),
        Commands::Build { backend } => commands::build::run(backend.as_deref()),
        Commands::Evaluate { backend } => commands::evaluate::run(backend.as_deref()).map(|_| ()),
        Commands::Run {
            backend,
            max_rounds,
            pause_after_plan,
            pause_after_eval,
            no_tui,
        } => commands::run::run(backend.as_deref(), max_rounds, pause_after_plan, pause_after_eval, no_tui),
        Commands::Status => commands::status::run(),
        Commands::Reset => commands::reset::run(),
        Commands::Feedback => commands::feedback::run(),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
