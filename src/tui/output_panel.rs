use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SyntectStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

/// An output line tagged with an optional agent source.
struct TaggedLine {
    agent: Option<String>,
    text: String,
}

/// View filter for the output panel.
enum ViewFilter {
    /// Show all lines from all agents.
    All,
    /// Show only lines from a specific agent.
    Agent(usize), // index into known_agents
}

pub struct OutputPanel {
    lines: Vec<TaggedLine>,
    /// Ordered list of agent names seen so far.
    known_agents: Vec<String>,
    filter: ViewFilter,
    scroll_offset: usize,
    follow_mode: bool,
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl OutputPanel {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            known_agents: Vec::new(),
            filter: ViewFilter::All,
            scroll_offset: 0,
            follow_mode: true,
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }

    /// Push a line with no agent tag.
    pub fn push_line(&mut self, line: String) {
        self.lines.push(TaggedLine {
            agent: None,
            text: line,
        });
        if self.follow_mode {
            self.scroll_offset = usize::MAX;
        }
    }

    /// Push a line tagged with an agent name.
    pub fn push_agent_line(&mut self, agent: &str, line: String) {
        // Track agent names in discovery order
        if !self.known_agents.iter().any(|a| a == agent) {
            self.known_agents.push(agent.to_string());
        }
        self.lines.push(TaggedLine {
            agent: Some(agent.to_string()),
            text: line,
        });
        if self.follow_mode {
            self.scroll_offset = usize::MAX;
        }
    }

    /// Cycle to the next view filter. Returns a description of the new filter.
    pub fn cycle_filter(&mut self) -> String {
        if self.known_agents.is_empty() {
            return "All".to_string();
        }
        self.filter = match &self.filter {
            ViewFilter::All => ViewFilter::Agent(0),
            ViewFilter::Agent(i) => {
                let next = i + 1;
                if next < self.known_agents.len() {
                    ViewFilter::Agent(next)
                } else {
                    ViewFilter::All
                }
            }
        };
        // Reset scroll on filter change
        self.scroll_offset = usize::MAX;
        self.follow_mode = true;
        self.current_filter_label()
    }

    /// Jump to a specific filter by index (0 = All, 1+ = agent index).
    pub fn set_filter(&mut self, index: usize) {
        if index == 0 {
            self.filter = ViewFilter::All;
        } else if index - 1 < self.known_agents.len() {
            self.filter = ViewFilter::Agent(index - 1);
        }
        self.scroll_offset = usize::MAX;
        self.follow_mode = true;
    }

    fn current_filter_label(&self) -> String {
        match &self.filter {
            ViewFilter::All => "All".to_string(),
            ViewFilter::Agent(i) => self
                .known_agents
                .get(*i)
                .cloned()
                .unwrap_or_else(|| "?".to_string()),
        }
    }

    /// Count filtered lines without borrowing the full vec.
    fn filtered_count(&self) -> usize {
        match &self.filter {
            ViewFilter::All => self.lines.len(),
            ViewFilter::Agent(idx) => {
                let name = match self.known_agents.get(*idx) {
                    Some(n) => n.as_str(),
                    None => return self.lines.len(),
                };
                self.lines
                    .iter()
                    .filter(|l| l.agent.as_deref() == Some(name) || l.agent.is_none())
                    .count()
            }
        }
    }

    /// Get the visible lines after filtering.
    fn filtered_lines(&self) -> Vec<&TaggedLine> {
        match &self.filter {
            ViewFilter::All => self.lines.iter().collect(),
            ViewFilter::Agent(idx) => {
                let name = match self.known_agents.get(*idx) {
                    Some(n) => n.as_str(),
                    None => return self.lines.iter().collect(),
                };
                self.lines
                    .iter()
                    .filter(|l| l.agent.as_deref() == Some(name) || l.agent.is_none())
                    .collect()
            }
        }
    }

    /// Get agent legend info for the status panel.
    pub fn legend(&self) -> super::status_panel::AgentLegend {
        super::status_panel::AgentLegend {
            agents: self.known_agents.clone(),
            filter: self.current_filter_label(),
        }
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.follow_mode = false;
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: usize, visible_height: usize) {
        let filtered = self.filtered_lines();
        let max = filtered.len().saturating_sub(visible_height);
        self.scroll_offset = (self.scroll_offset + amount).min(max);
        if self.scroll_offset >= max {
            self.follow_mode = true;
        }
    }

    pub fn toggle_follow(&mut self) {
        self.follow_mode = !self.follow_mode;
        if self.follow_mode {
            self.scroll_offset = usize::MAX;
        }
    }

    pub fn page_up(&mut self, visible_height: usize) {
        self.scroll_up(visible_height.saturating_sub(2));
    }

    pub fn page_down(&mut self, visible_height: usize) {
        self.scroll_down(visible_height.saturating_sub(2), visible_height);
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let inner_height = area.height.saturating_sub(2) as usize;

        // Compute filtered count and clamp scroll before borrowing filtered lines
        let filtered_count = self.filtered_count();
        let max_scroll = filtered_count.saturating_sub(inner_height);
        if self.follow_mode || self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }
        let scroll = self.scroll_offset;

        let filtered = self.filtered_lines();
        let visible = &filtered[scroll..filtered.len().min(scroll + inner_height)];
        let multi_agent = self.known_agents.len() > 1;
        let agents_snapshot = self.known_agents.clone();

        let styled_lines: Vec<Line> = visible
            .iter()
            .map(|tl| {
                if multi_agent {
                    if let Some(ref agent) = tl.agent {
                        let color = agent_color(agent, &agents_snapshot);
                        let tag = Span::styled(
                            format!("[{agent}] "),
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        );
                        let rest = self.highlight_line(&tl.text);
                        let mut spans = vec![tag];
                        spans.extend(rest.spans);
                        Line::from(spans)
                    } else {
                        self.highlight_line(&tl.text)
                    }
                } else {
                    self.highlight_line(&tl.text)
                }
            })
            .collect();

        let follow_indicator = if self.follow_mode { "FOLLOW" } else { "SCROLL" };
        let filter_label = self.current_filter_label();
        let filter_hint = if multi_agent {
            format!(" | Filter: {filter_label} (` to cycle)")
        } else {
            String::new()
        };
        let title = format!(" Live Output [{follow_indicator}]{filter_hint} ");

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let paragraph = Paragraph::new(styled_lines).block(block);
        frame.render_widget(paragraph, area);
    }

    fn highlight_line<'a>(&self, line: &str) -> Line<'a> {
        if line.trim_start().starts_with("```") {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
        }

        if line.contains("error") || line.contains("Error") || line.contains("ERROR") {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Red),
            ));
        }

        if line.contains("warning") || line.contains("Warning") || line.contains("WARN") {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Yellow),
            ));
        }

        if (line.contains('/') || line.contains('\\')) && line.contains('.') && !line.contains(' ')
        {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Cyan),
            ));
        }

        if line.starts_with("commit ")
            || line.starts_with("diff ")
            || line.starts_with("+++")
            || line.starts_with("---")
        {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Magenta),
            ));
        }

        if looks_like_code(line)
            && let Some(highlighted) = self.syntect_highlight(line)
        {
            return highlighted;
        }

        Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::White),
        ))
    }

    fn syntect_highlight<'a>(&self, line: &str) -> Option<Line<'a>> {
        let theme = &self.theme_set.themes["base16-ocean.dark"];
        let syntax = self
            .syntax_set
            .find_syntax_by_extension("rs")
            .or_else(|| Some(self.syntax_set.find_syntax_plain_text()))?;
        let mut h = HighlightLines::new(syntax, theme);
        let ranges = h.highlight_line(line, &self.syntax_set).ok()?;

        let spans: Vec<Span> = ranges
            .into_iter()
            .map(|(style, text)| Span::styled(text.to_string(), syntect_to_ratatui_style(style)))
            .collect();

        Some(Line::from(spans))
    }
}

/// Assign a consistent color to an agent name.
fn agent_color(name: &str, agents: &[String]) -> Color {
    let colors = [
        Color::LightCyan,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightMagenta,
        Color::LightBlue,
        Color::LightRed,
    ];
    let idx = agents.iter().position(|a| a == name).unwrap_or(0);
    colors[idx % colors.len()]
}

fn looks_like_code(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.ends_with(';')
        || trimmed.ends_with('{')
        || trimmed.ends_with('}')
        || trimmed.starts_with("fn ")
        || trimmed.starts_with("let ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("pub ")
        || trimmed.starts_with("impl ")
        || trimmed.starts_with("struct ")
        || trimmed.starts_with("enum ")
        || trimmed.starts_with("mod ")
        || trimmed.starts_with("def ")
        || trimmed.starts_with("class ")
        || trimmed.starts_with("import ")
        || trimmed.starts_with("from ")
        || trimmed.starts_with("const ")
        || trimmed.contains("->")
        || trimmed.contains("=>")
}

fn syntect_to_ratatui_style(style: SyntectStyle) -> Style {
    Style::default().fg(Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    ))
}
