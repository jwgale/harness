mod agents;
mod artifacts;
mod bridge;
mod cli_backend;
mod commands;
mod config;
mod evaluator;
mod global_config;
mod notifications;
mod plugins;
mod progress;
mod prompts;
mod scl;
mod scl_lifecycle;
mod tui;
mod vault;
mod workflows;
mod xdg;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "harness",
    version,
    about = "Orchestrate planner → builder → evaluator loops using subscription CLI tools"
)]
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
        /// Comma-separated agent names to run sequentially (e.g. "planner,builder,evaluator")
        #[arg(long)]
        agents: Option<String>,
        /// Named workflow to run (from ~/.config/harness/workflows/)
        #[arg(long)]
        workflow: Option<String>,
        /// Run --agents in parallel (concurrent execution)
        #[arg(long)]
        parallel: bool,
    },
    /// Print current harness state from artifacts
    Status,
    /// Diagnose local Harness, backend, and toolchain readiness
    Doctor {
        /// Also run fmt, clippy, and tests
        #[arg(long)]
        deep: bool,
    },
    /// Generate handoff.md for context reset
    Reset,
    /// Show latest evaluator feedback
    Feedback,
    /// Manage the persistent daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Manage daemon workspaces
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    /// Manage scheduled tasks
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
    /// Shared Context Layer integration
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },
    /// Manage evaluator strategies
    Evaluator {
        #[command(subcommand)]
        action: EvaluatorAction,
    },
    /// Manage agent definitions
    Agent {
        #[command(subcommand)]
        action: AgentAction,
    },
    /// Manage and validate workflows
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction2,
    },
    /// SanctumAI credential vault
    Vault {
        #[command(subcommand)]
        action: VaultAction,
    },
    /// External service bridges (Telegram, etc.)
    Bridge {
        #[command(subcommand)]
        action: BridgeAction,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Start the daemon as a systemd user service
    Start,
    /// Stop the daemon
    Stop,
    /// Show daemon status
    Status,
    /// Show recent daemon logs
    Logs,
    /// Internal: run the daemon loop (used by systemd)
    #[command(hide = true)]
    InternalRun,
}

#[derive(Subcommand)]
enum PluginAction {
    /// List installed plugins
    List,
}

#[derive(Subcommand)]
enum ScheduleAction {
    /// Add a scheduled task (cron-style)
    Add {
        /// Name for this schedule
        name: String,
        /// Cron expression (5 fields: min hour dom mon dow)
        cron: String,
        /// Command to execute
        command: String,
    },
    /// List scheduled tasks
    List,
    /// Remove a scheduled task
    Remove {
        /// Name of the schedule to remove
        name: String,
    },
    /// Manually trigger a scheduled task now
    Run {
        /// Name of the schedule to run
        name: String,
    },
    /// Show schedule execution history
    History {
        /// Number of entries to show
        #[arg(long, default_value = "20")]
        limit: u32,
    },
}

#[derive(Subcommand)]
enum WorkspaceAction {
    /// Register a project directory for daemon monitoring (default: current dir)
    Register {
        /// Path to the project directory (default: current dir)
        path: Option<String>,
    },
    /// List registered workspaces
    List,
    /// Remove a registered workspace
    Remove {
        /// Name of the workspace to remove
        name: String,
    },
}

#[derive(Subcommand)]
enum VaultAction {
    /// Initialize vault configuration and generate signing key
    Init,
    /// Show vault connection status
    Status,
    /// Store a credential in the vault
    #[command(name = "add")]
    CredentialAdd {
        /// Credential path/name (e.g. "slack/webhook-url")
        name: String,
    },
    /// List credentials in the vault
    #[command(name = "list")]
    CredentialList,
}

#[derive(Subcommand)]
enum WorkflowAction2 {
    /// List all defined workflows
    List,
    /// Validate a workflow definition
    Validate {
        /// Workflow name
        name: String,
    },
}

#[derive(Subcommand)]
enum AgentAction {
    /// List all defined agents
    List,
    /// Add a new agent definition
    Add {
        /// Agent name
        name: String,
        /// Role: planner, builder, evaluator, or custom
        #[arg(long)]
        role: String,
        /// Backend: claude, codex, or mock
        #[arg(long)]
        backend: String,
        /// Optional description
        #[arg(long)]
        description: Option<String>,
    },
    /// Remove an agent definition
    Remove {
        /// Agent name
        name: String,
    },
}

#[derive(Subcommand)]
enum EvaluatorAction {
    /// List available evaluator strategies
    List,
    /// Set the evaluator strategy for this workspace
    Use {
        /// Strategy name: default, playwright-mcp, or curl
        name: String,
    },
}

#[derive(Subcommand)]
enum BridgeAction {
    /// Telegram bot bridge
    Telegram {
        #[command(subcommand)]
        action: TelegramAction,
    },
}

#[derive(Subcommand)]
enum TelegramAction {
    /// Start the Telegram bridge (systemd-managed)
    Start,
    /// Stop the Telegram bridge
    Stop,
    /// Show Telegram bridge status
    Status,
    /// Internal: run the listener loop (used by systemd)
    #[command(hide = true)]
    InternalListen,
}

#[derive(Subcommand)]
enum ContextAction {
    /// Show SCL connection status
    Status,
    /// Query the shared context layer
    Query {
        /// Query text
        query: String,
    },
    /// Record an entry to the shared context layer
    Record {
        /// Kind: architecture, decision, convention, active_work, insight, gotcha
        kind: String,
        /// Content to record
        content: String,
    },
}

fn main() {
    // Ensure global config exists on first run
    let _ = global_config::ensure_global_config();

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
            agents,
            workflow,
            parallel,
        } => {
            if agents.is_some() || workflow.is_some() {
                commands::run::run_multi_agent(
                    backend.as_deref(),
                    max_rounds,
                    agents.as_deref(),
                    workflow.as_deref(),
                    parallel,
                    no_tui,
                )
            } else {
                commands::run::run(
                    backend.as_deref(),
                    max_rounds,
                    pause_after_plan,
                    pause_after_eval,
                    no_tui,
                )
            }
        }
        Commands::Status => commands::status::run(),
        Commands::Doctor { deep } => commands::doctor::run(deep),
        Commands::Reset => commands::reset::run(),
        Commands::Feedback => commands::feedback::run(),
        Commands::Daemon { action } => match action {
            DaemonAction::Start => commands::daemon::run("start"),
            DaemonAction::Stop => commands::daemon::run("stop"),
            DaemonAction::Status => commands::daemon::run("status"),
            DaemonAction::Logs => commands::daemon::run("logs"),
            DaemonAction::InternalRun => commands::daemon::run_daemon_loop(),
        },
        Commands::Plugin { action } => match action {
            PluginAction::List => plugins::list(),
        },
        Commands::Workspace { action } => match action {
            WorkspaceAction::Register { path } => commands::workspace::register(path.as_deref()),
            WorkspaceAction::List => commands::workspace::list(),
            WorkspaceAction::Remove { name } => commands::workspace::remove(&name),
        },
        Commands::Schedule { action } => match action {
            ScheduleAction::Add {
                name,
                cron,
                command,
            } => commands::schedule::add(&name, &cron, &command),
            ScheduleAction::List => commands::schedule::list(),
            ScheduleAction::Remove { name } => commands::schedule::remove(&name),
            ScheduleAction::Run { name } => commands::schedule::run_now(&name),
            ScheduleAction::History { limit } => commands::schedule::history(limit),
        },
        Commands::Context { action } => match action {
            ContextAction::Status => commands::context::status(),
            ContextAction::Query { query } => commands::context::query(&query),
            ContextAction::Record { kind, content } => commands::context::record(&kind, &content),
        },
        Commands::Evaluator { action } => match action {
            EvaluatorAction::List => commands::evaluator_cmd::list(),
            EvaluatorAction::Use { name } => commands::evaluator_cmd::use_strategy(&name),
        },
        Commands::Agent { action } => match action {
            AgentAction::List => commands::agent_cmd::list(),
            AgentAction::Add {
                name,
                role,
                backend,
                description,
            } => commands::agent_cmd::add(&name, &role, &backend, description.as_deref()),
            AgentAction::Remove { name } => commands::agent_cmd::remove(&name),
        },
        Commands::Workflow { action } => match action {
            WorkflowAction2::List => commands::workflow_cmd::list(),
            WorkflowAction2::Validate { name } => commands::workflow_cmd::validate(&name),
        },
        Commands::Bridge { action } => match action {
            BridgeAction::Telegram { action } => match action {
                TelegramAction::Start => commands::bridge_cmd::start(),
                TelegramAction::Stop => commands::bridge_cmd::stop(),
                TelegramAction::Status => commands::bridge_cmd::status(),
                TelegramAction::InternalListen => commands::bridge_cmd::internal_listen(),
            },
        },
        Commands::Vault { action } => match action {
            VaultAction::Init => commands::vault_cmd::init(),
            VaultAction::Status => commands::vault_cmd::status(),
            VaultAction::CredentialAdd { name } => commands::vault_cmd::credential_add(&name),
            VaultAction::CredentialList => commands::vault_cmd::credential_list(),
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
