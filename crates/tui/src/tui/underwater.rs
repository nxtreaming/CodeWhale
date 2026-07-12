//! Coherent shell grammar for the underwater TUI.
//!
//! This module owns phase, responsive density, the empty-state composition,
//! and the compact header/footer fact budget. Product data still belongs to
//! [`App`]; this is only its terminal projection. Keeping these decisions in
//! one place prevents the default UI from drifting back into a header +
//! sidebar + dashboard + footer composition with four owners for one fact.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
};
use unicode_width::UnicodeWidthStr;

use crate::tui::{
    app::{App, AppMode},
    views::ModalKind,
};

/// Responsive density tier. It changes how much truth is shown, never the
/// underlying state grammar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellTier {
    Compact,
    Normal,
    Wide,
}

impl ShellTier {
    #[must_use]
    pub fn for_area(area: Rect) -> Self {
        if area.width < 60 || area.height < 16 {
            Self::Compact
        } else if area.width < 110 || area.height < 30 {
            Self::Normal
        } else {
            Self::Wide
        }
    }

    #[must_use]
    fn for_chrome_width(width: u16) -> Self {
        if width < 60 {
            Self::Compact
        } else if width < 110 {
            Self::Normal
        } else {
            Self::Wide
        }
    }
}

/// Perceptual session phase. Every treatment reads from this same enum so a
/// footer cannot say `idle` while the transcript is asking for approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellPhase {
    Idle,
    Typing,
    Working,
    Waiting,
    Approval,
    Done,
    Failed,
}

impl ShellPhase {
    #[must_use]
    pub fn from_app(app: &App) -> Self {
        if matches!(
            app.view_stack.top_kind(),
            Some(
                ModalKind::Approval
                    | ModalKind::Elevation
                    | ModalKind::UserInput
                    | ModalKind::PlanPrompt
            )
        ) {
            return Self::Approval;
        }
        if app.turn_error_posted
            || matches!(app.runtime_turn_status.as_deref(), Some("failed" | "error"))
        {
            return Self::Failed;
        }
        if app.pending_user_input_prompt.is_some()
            || app.plan_prompt_pending
            || app
                .task_panel
                .iter()
                .any(|task| matches!(task.status.as_str(), "waiting" | "needs_user"))
        {
            return Self::Waiting;
        }
        if app.is_loading
            || matches!(app.runtime_turn_status.as_deref(), Some("in_progress"))
            || app
                .active_cell
                .as_ref()
                .is_some_and(|active| !active.is_empty())
        {
            return Self::Working;
        }
        if matches!(app.runtime_turn_status.as_deref(), Some("completed")) {
            return Self::Done;
        }
        if !app.input.is_empty() {
            return Self::Typing;
        }
        Self::Idle
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Typing => "draft",
            Self::Working => "working",
            Self::Waiting | Self::Approval => "waiting on you",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn color(self, app: &App) -> Color {
        match self {
            Self::Idle | Self::Done => app.ui_theme.text_muted,
            Self::Typing => app.ui_theme.accent_primary,
            Self::Working => app.ui_theme.status_working,
            Self::Waiting | Self::Approval | Self::Failed => app.ui_theme.error_fg,
        }
    }
}

fn mode_label(mode: AppMode) -> &'static str {
    match mode {
        AppMode::Agent | AppMode::Auto | AppMode::Yolo => "act",
        AppMode::Plan => "plan",
        AppMode::Operate => "operate",
    }
}

fn permission_label(app: &App) -> &'static str {
    if app.mode == AppMode::Plan {
        "read only"
    } else {
        match app.approval_mode.permission_chip_label() {
            "Ask" => "ask",
            "Auto-Review" => "auto",
            "Full Access" => "bypass",
            "Never" => "never",
            _ => "ask",
        }
    }
}

fn span_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.width()).sum()
}

fn truncate_to_width(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width <= 3 {
        return ".".repeat(width);
    }
    let mut result = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width + 1 > width {
            break;
        }
        result.push(ch);
        used += ch_width;
    }
    result.push('…');
    result
}

fn compact_tokens(tokens: i64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.0}K", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

/// Render the one-line shell header. Route, mode, permission, active-agent
/// count, and context each have exactly one owner here.
pub fn render_header(area: Rect, buf: &mut Buffer, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let tier = ShellTier::for_chrome_width(area.width);
    Block::default()
        .style(Style::default().bg(app.ui_theme.header_bg))
        .render(area, buf);

    let mut left = vec![
        Span::styled(
            "cw",
            Style::default()
                .fg(app.ui_theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            app.model_display_label(),
            Style::default().fg(app.ui_theme.text_muted),
        ),
        Span::styled(" · ", Style::default().fg(app.ui_theme.text_dim)),
        Span::styled(
            mode_label(app.mode),
            Style::default().fg(match app.mode {
                AppMode::Plan => app.ui_theme.mode_plan,
                AppMode::Operate => app.ui_theme.mode_operate,
                _ => app.ui_theme.mode_agent,
            }),
        ),
    ];
    if tier != ShellTier::Compact {
        left.push(Span::styled(
            " · ",
            Style::default().fg(app.ui_theme.text_dim),
        ));
        left.push(Span::styled(
            permission_label(app),
            Style::default().fg(app.ui_theme.text_muted),
        ));
    }

    let running_agents = crate::tui::subagent_routing::running_agent_count(app);
    let mut right = Vec::new();
    if tier == ShellTier::Wide && running_agents > 0 {
        right.push(Span::styled(
            format!("agents {running_agents}"),
            Style::default().fg(app.ui_theme.text_muted),
        ));
        right.push(Span::styled(
            " · ",
            Style::default().fg(app.ui_theme.text_dim),
        ));
    }
    if tier != ShellTier::Compact
        && let Some((used, max, percent)) = crate::tui::ui::context_usage_snapshot(app)
    {
        let filled = ((percent / 100.0) * 5.0).ceil().clamp(0.0, 5.0) as usize;
        right.push(Span::styled(
            format!(
                "{}/{} [{}{}] {:.0}%",
                compact_tokens(used),
                compact_tokens(i64::from(max)),
                "▰".repeat(filled),
                "▱".repeat(5usize.saturating_sub(filled)),
                percent
            ),
            Style::default().fg(app.ui_theme.info),
        ));
    }
    if tier == ShellTier::Wide {
        if !right.is_empty() {
            right.push(Span::raw("  "));
        }
        right.push(Span::styled(
            format!("v{}", env!("CARGO_PKG_VERSION")),
            Style::default().fg(app.ui_theme.text_hint),
        ));
    }

    let available = usize::from(area.width);
    let right_width = span_width(&right);
    let left_budget = available.saturating_sub(right_width + usize::from(right_width > 0));
    if span_width(&left) > left_budget {
        left = vec![
            Span::styled(
                "cw",
                Style::default()
                    .fg(app.ui_theme.accent_primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                truncate_to_width(&app.model_display_label(), left_budget.saturating_sub(7)),
                Style::default().fg(app.ui_theme.text_muted),
            ),
            Span::styled(" · ", Style::default().fg(app.ui_theme.text_dim)),
            Span::styled(
                mode_label(app.mode),
                Style::default().fg(app.ui_theme.accent_primary),
            ),
        ];
    }
    let left_width = span_width(&left);
    let gap = available.saturating_sub(left_width + right_width);
    left.push(Span::raw(" ".repeat(gap)));
    left.extend(right);
    let title_area = Rect { height: 1, ..area };
    Paragraph::new(Line::from(left)).render(title_area, buf);
    if area.height > 1 {
        let rule_area = Rect {
            y: area.y.saturating_add(1),
            height: 1,
            ..area
        };
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(usize::from(area.width)),
            Style::default().fg(app.ui_theme.border),
        )))
        .render(rule_area, buf);
    }
}

/// Render the fixed one-line footer. It owns phase, cost, and the keys that
/// open detail; route, permission, repository, MCP, and context do not repeat.
pub fn render_footer(area: Rect, buf: &mut Buffer, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let phase = ShellPhase::from_app(app);
    let tier = ShellTier::for_chrome_width(area.width);
    Block::default()
        .style(Style::default().bg(app.ui_theme.footer_bg))
        .render(area, buf);

    let mut left = vec![Span::styled(
        phase.label(),
        Style::default().fg(phase.color(app)).add_modifier(
            if matches!(phase, ShellPhase::Waiting | ShellPhase::Approval) {
                Modifier::BOLD
            } else {
                Modifier::empty()
            },
        ),
    )];
    let cost = app.displayed_session_cost_for_currency(app.cost_currency);
    if cost > 0.000_001 && tier != ShellTier::Compact {
        left.push(Span::styled(
            " · ",
            Style::default().fg(app.ui_theme.text_dim),
        ));
        left.push(Span::styled(
            app.format_cost_amount(cost),
            Style::default().fg(app.ui_theme.text_muted),
        ));
    }

    let right_text = match tier {
        ShellTier::Compact => "Alt+?:keys",
        ShellTier::Normal => "v:output · Alt+?:keys",
        ShellTier::Wide => "v:output · Alt+C:context · Alt+?:keys",
    };
    let right_width = right_text.width();
    let available = usize::from(area.width);
    let left_width = span_width(&left);
    if left_width + right_width < available {
        left.push(Span::raw(" ".repeat(available - left_width - right_width)));
        left.push(Span::styled(
            right_text,
            Style::default().fg(app.ui_theme.text_hint),
        ));
    }
    Paragraph::new(Line::from(left)).render(area, buf);
}

/// Build the post-launch idle composition. It is deliberately not a command
/// dashboard: one brand mark, one context line, and one quiet Fleet setup path.
pub fn empty_state_lines(app: &App, area: Rect) -> Vec<Line<'static>> {
    if area.width == 0 || area.height == 0 {
        return Vec::new();
    }
    let width = usize::from(area.width);
    let tier = ShellTier::for_area(area);
    let mut lines = vec![Line::from(""); usize::from(area.height / 4)];
    if tier != ShellTier::Compact && area.height >= 14 && area.width >= 28 {
        let mark = [
            "   ˚",
            " ▗▄▄▄▄▄▄▄▄▄▄▄▄▄▖    ▚▞",
            "▐██·████████████▙▄▄▄▞",
            " ▝▀▀▀▀▀▀▀▀▀▀▀▀▀▘",
        ];
        for row in mark {
            let inset = " ".repeat(width.saturating_sub(row.width()) / 2);
            lines.push(Line::from(Span::styled(
                format!("{inset}{row}"),
                Style::default().fg(app.ui_theme.accent_primary),
            )));
        }
        lines.push(Line::from(""));
    }

    let identity = crate::tui::workspace_context::identity_from_context(
        &app.workspace,
        app.workspace_context.as_deref(),
    );
    let workspace = crate::utils::display_path(&app.workspace);
    let branch = identity.branch.as_deref().unwrap_or("no git");
    let context = if tier == ShellTier::Compact {
        format!("codewhale · {branch}")
    } else {
        format!(
            "codewhale · {workspace} · {branch} · mcp {}",
            app.mcp_configured_count
        )
    };
    let context = truncate_to_width(&context, width);
    let inset = " ".repeat(width.saturating_sub(context.width()) / 2);
    lines.push(Line::from(Span::styled(
        format!("{inset}{context}"),
        Style::default().fg(app.ui_theme.text_muted),
    )));
    if area.height >= 6 {
        lines.push(Line::from(""));
        let fleet = if tier == ShellTier::Compact {
            "Fleet  /fleet setup"
        } else {
            "Fleet setup  /fleet setup"
        };
        let inset = " ".repeat(width.saturating_sub(fleet.width()) / 2);
        lines.push(Line::from(Span::styled(
            format!("{inset}{fleet}"),
            Style::default().fg(app.ui_theme.text_hint),
        )));
    }
    lines
}
