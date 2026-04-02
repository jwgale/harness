use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use syntect::highlighting::{ThemeSet, Style as SyntectStyle};
use syntect::parsing::SyntaxSet;
use syntect::easy::HighlightLines;

pub struct OutputPanel {
    lines: Vec<String>,
    scroll_offset: usize,
    follow_mode: bool,
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
}

impl OutputPanel {
    pub fn new() -> Self {
        Self {
            lines: Vec::new(),
            scroll_offset: 0,
            follow_mode: true,
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
        }
    }

    pub fn push_line(&mut self, line: String) {
        self.lines.push(line);
        if self.follow_mode {
            // Will be clamped during render
            self.scroll_offset = usize::MAX;
        }
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.follow_mode = false;
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: usize, visible_height: usize) {
        let max = self.lines.len().saturating_sub(visible_height);
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
        let inner_height = area.height.saturating_sub(2) as usize; // borders

        // Clamp scroll
        let max_scroll = self.lines.len().saturating_sub(inner_height);
        if self.follow_mode || self.scroll_offset > max_scroll {
            self.scroll_offset = max_scroll;
        }

        let visible_lines = &self.lines[self.scroll_offset..self.lines.len().min(self.scroll_offset + inner_height)];

        let styled_lines: Vec<Line> = visible_lines.iter().map(|l| self.highlight_line(l)).collect();

        let follow_indicator = if self.follow_mode { " [FOLLOW] " } else { " [SCROLL] " };
        let title = format!(" Live Output {follow_indicator}");

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let paragraph = Paragraph::new(styled_lines).block(block);
        frame.render_widget(paragraph, area);
    }

    fn highlight_line<'a>(&self, line: &str) -> Line<'a> {
        // Detect code block markers
        if line.trim_start().starts_with("```") {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Error lines
        if line.contains("error") || line.contains("Error") || line.contains("ERROR") {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Red),
            ));
        }

        // Warning lines
        if line.contains("warning") || line.contains("Warning") || line.contains("WARN") {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Yellow),
            ));
        }

        // File paths (contains / and a file extension)
        if (line.contains('/') || line.contains('\\')) && line.contains('.') && !line.contains(' ') {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Cyan),
            ));
        }

        // Git output
        if line.starts_with("commit ") || line.starts_with("diff ") || line.starts_with("+++") || line.starts_with("---") {
            return Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Magenta),
            ));
        }

        // Try syntect highlighting for code-looking lines
        if looks_like_code(line)
            && let Some(highlighted) = self.syntect_highlight(line)
        {
            return highlighted;
        }

        // Default
        Line::from(Span::styled(
            line.to_string(),
            Style::default().fg(Color::White),
        ))
    }

    fn syntect_highlight<'a>(&self, line: &str) -> Option<Line<'a>> {
        let theme = &self.theme_set.themes["base16-ocean.dark"];
        let syntax = self.syntax_set.find_syntax_by_extension("rs")
            .or_else(|| Some(self.syntax_set.find_syntax_plain_text()))?;
        let mut h = HighlightLines::new(syntax, theme);
        let ranges = h.highlight_line(line, &self.syntax_set).ok()?;

        let spans: Vec<Span> = ranges.into_iter().map(|(style, text)| {
            Span::styled(
                text.to_string(),
                syntect_to_ratatui_style(style),
            )
        }).collect();

        Some(Line::from(spans))
    }
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
