use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::spec_parser::{Feature, FeatureStatus};
use super::TuiPhase;

/// Agent legend info for multi-agent TUI display.
pub struct AgentLegend {
    pub agents: Vec<String>,
    pub filter: String,
}

impl AgentLegend {
    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self { agents: Vec::new(), filter: "All".to_string() }
    }
}

pub struct EvalScores {
    pub functionality: Option<u32>,
    pub completeness: Option<u32>,
    pub code_quality: Option<u32>,
    pub design_quality: Option<u32>,
    pub robustness: Option<u32>,
    pub verdict: Option<String>,
}

impl EvalScores {
    pub fn empty() -> Self {
        Self {
            functionality: None,
            completeness: None,
            code_quality: None,
            design_quality: None,
            robustness: None,
            verdict: None,
        }
    }

    /// Parse evaluation scores from evaluator output.
    pub fn parse(output: &str) -> Self {
        let mut scores = Self::empty();
        for line in output.lines() {
            let trimmed = line.trim().to_lowercase();
            if let Some(val) = extract_score(&trimmed, "functionality") {
                scores.functionality = Some(val);
            } else if let Some(val) = extract_score(&trimmed, "completeness") {
                scores.completeness = Some(val);
            } else if let Some(val) = extract_score(&trimmed, "code_quality") {
                scores.code_quality = Some(val);
            } else if let Some(val) = extract_score(&trimmed, "design_quality") {
                scores.design_quality = Some(val);
            } else if let Some(val) = extract_score(&trimmed, "robustness") {
                scores.robustness = Some(val);
            }
            if let Some(rest) = trimmed.strip_prefix("verdict:") {
                scores.verdict = Some(rest.trim().to_uppercase());
            }
        }
        scores
    }
}

fn extract_score(line: &str, key: &str) -> Option<u32> {
    if !line.contains(key) {
        return None;
    }
    // Look for patterns like "functionality: 8/10" or "functionality: 8"
    let after_key = line.split(key).nth(1)?;
    let after_colon = after_key.strip_prefix(':')?;
    let trimmed = after_colon.trim();
    // Handle "8/10" or just "8"
    let num_str = trimmed.split('/').next()?;
    num_str.trim().parse().ok()
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    project_name: &str,
    phase: &TuiPhase,
    round: u32,
    max_rounds: u32,
    backend: &str,
    elapsed_secs: u64,
    features: &[Feature],
    scores: &EvalScores,
    legend: &AgentLegend,
) {
    let mut lines: Vec<Line> = Vec::new();

    // Project info
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Project: ", Style::default().fg(Color::DarkGray)),
        Span::styled(project_name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Phase:   ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("{} (round {}/{})", phase.label(), round, max_rounds),
            Style::default().fg(phase.color()).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Backend: ", Style::default().fg(Color::DarkGray)),
        Span::styled(backend, Style::default().fg(Color::Cyan)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  Elapsed: ", Style::default().fg(Color::DarkGray)),
        Span::styled(format_elapsed(elapsed_secs), Style::default().fg(Color::White)),
    ]));

    // Multi-agent info
    match phase {
        TuiPhase::Parallel(names) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "  ── Parallel Agents ──",
                Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            )));
            for name in names {
                lines.push(Line::from(vec![
                    Span::raw("  "),
                    Span::styled("⟩ ", Style::default().fg(Color::Blue)),
                    Span::styled(name.clone(), Style::default().fg(Color::White)),
                ]));
            }
        }
        TuiPhase::Loop { round: lr, max: lm } => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  Loop:    ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("iteration {lr}/{lm}"),
                    Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        TuiPhase::AgentStep(name, role) => {
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("  Agent:   ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{name} [{role}]"),
                    Style::default().fg(Color::LightCyan).add_modifier(Modifier::BOLD),
                ),
            ]));
        }
        _ => {}
    }

    // Agent legend (multi-agent mode)
    if !legend.agents.is_empty() {
        let agent_colors = [
            Color::LightCyan, Color::LightGreen, Color::LightYellow,
            Color::LightMagenta, Color::LightBlue, Color::LightRed,
        ];
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500} Agents \u{2500}\u{2500}\u{2500}",
            Style::default().fg(Color::DarkGray),
        )));
        for (i, name) in legend.agents.iter().enumerate() {
            let color = agent_colors[i % agent_colors.len()];
            let key_hint = if i < 4 { format!(" ({})", i + 4) } else { String::new() };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("\u{25cf} ", Style::default().fg(color)),
                Span::styled(name.clone(), Style::default().fg(color)),
                Span::styled(key_hint, Style::default().fg(Color::DarkGray)),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled("  Filter: ", Style::default().fg(Color::DarkGray)),
            Span::styled(&legend.filter, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::styled("  ` cycle", Style::default().fg(Color::DarkGray)),
        ]));
    }

    // Features
    if !features.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500} Spec Features \u{2500}\u{2500}\u{2500}",
            Style::default().fg(Color::DarkGray),
        )));
        for f in features {
            let (icon, color) = match f.status {
                FeatureStatus::NotStarted => ("[ ]", Color::DarkGray),
                FeatureStatus::InProgress => ("[~]", Color::Yellow),
                FeatureStatus::Completed => ("[x]", Color::Green),
            };
            let name = if f.name.len() > 28 {
                format!("{}...", &f.name[..25])
            } else {
                f.name.clone()
            };
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(icon, Style::default().fg(color)),
                Span::styled(format!(" {name}"), Style::default().fg(color)),
            ]));
        }
    }

    // Evaluation scores
    if scores.functionality.is_some() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "  \u{2500}\u{2500}\u{2500} Last Evaluation \u{2500}\u{2500}\u{2500}",
            Style::default().fg(Color::DarkGray),
        )));
        if let Some(v) = scores.functionality {
            lines.push(score_line("  functionality", v));
        }
        if let Some(v) = scores.completeness {
            lines.push(score_line("  completeness", v));
        }
        if let Some(v) = scores.code_quality {
            lines.push(score_line("  code_quality", v));
        }
        if let Some(v) = scores.design_quality {
            lines.push(score_line("  design_quality", v));
        }
        if let Some(v) = scores.robustness {
            lines.push(score_line("  robustness", v));
        }
        if let Some(ref v) = scores.verdict {
            let color = match v.as_str() {
                "PASS" => Color::Green,
                "FAIL" => Color::Red,
                _ => Color::Yellow,
            };
            lines.push(Line::from(vec![
                Span::styled("  VERDICT: ", Style::default().fg(Color::DarkGray)),
                Span::styled(v.clone(), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            ]));
        }
    }

    let title = if legend.agents.is_empty() {
        " Harness Status ".to_string()
    } else {
        " Harness Multi-Agent ".to_string()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, area);
}

fn score_line(label: &str, value: u32) -> Line<'static> {
    let color = if value >= 8 {
        Color::Green
    } else if value >= 6 {
        Color::Yellow
    } else {
        Color::Red
    };
    Line::from(vec![
        Span::styled(format!("{label}: "), Style::default().fg(Color::DarkGray)),
        Span::styled(format!("{value}/10"), Style::default().fg(color)),
    ])
}

fn format_elapsed(secs: u64) -> String {
    let m = secs / 60;
    let s = secs % 60;
    if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}
