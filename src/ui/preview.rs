//! The header line and the selected theme's sample terminal and palette.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Padding, Paragraph};

use super::{pretty_name, rgb};
use crate::app::App;
use crate::repo::Theme;

/// One line on top: what's applied now, and what's switched on.
pub(super) fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let name = |ghostty: &str| ghostty.trim().trim_start_matches("Omarchy ").to_string();
    let mut left = vec![Span::raw(" oms ").blue().bold().reversed(), Span::raw("  ")];
    match app.applied_theme.as_deref() {
        Some(theme) if theme.contains("light:") || theme.contains("dark:") => {
            for part in theme.split(',') {
                if let Some(light) = part.trim().strip_prefix("light:") {
                    left.push(Span::raw("☀ ").yellow());
                    left.push(Span::raw(format!("{}  ", name(light))));
                } else if let Some(dark) = part.trim().strip_prefix("dark:") {
                    left.push(Span::raw("☾ ").yellow());
                    left.push(Span::raw(format!("{}  ", name(dark))));
                }
            }
        }
        Some(theme) => {
            left.push(Span::raw("● ").green());
            left.push(Span::raw(format!("{}  ", name(theme))));
        }
        None => left.push(Span::raw("no theme applied yet  ").dark_gray()),
    }
    if let Some(path) = &app.applied_wallpaper {
        left.push(Span::raw("▣ ").dark_gray());
        left.push(Span::raw(pretty_name(path)).dark_gray());
    }
    let mut right = Vec::new();
    if let Some(minutes) = app.settings.rotate_minutes {
        let every = if minutes % 60 == 0 {
            format!("{}h", minutes / 60)
        } else {
            format!("{minutes}m")
        };
        right.push(Span::raw(format!("↻ every {every}  ")).dark_gray());
    }
    if !app.settings.apps.is_empty() {
        right.push(Span::raw(format!("+ {} ", app.settings.apps.join(" "))).dark_gray());
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);
    f.render_widget(Paragraph::new(Line::from(right).right_aligned()), area);
}

/// Width of the sample terminal text, in cells.
const SAMPLE_WIDTH: u16 = 52;
/// Width of the palette grid: 8 swatches of 4 cells with 1-cell gaps.
const GRID_WIDTH: u16 = 8 * 5 - 1;

/// The selected theme: a sample terminal and its 16 colors. `compact` shows
/// only the colors.
pub(super) fn draw_preview(f: &mut Frame, t: &Theme, area: Rect, compact: bool) {
    let bg = rgb(t.background);
    let mode = if t.is_light() { "light" } else { "dark" };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(rgb(t.palette[8])).bg(bg))
        .title(Line::from(format!(" {} ", t.name)).style(Style::new().fg(rgb(t.palette[4])).bold()))
        .title(
            Line::from(format!(" {mode} "))
                .right_aligned()
                .style(Style::new().fg(rgb(t.palette[8]))),
        )
        .padding(Padding::horizontal(1))
        .style(Style::new().bg(bg).fg(rgb(t.foreground)));

    // Strips of swatches that shrink to fit narrow panes.
    let swatch_width = (area.width.saturating_sub(4) / 8).clamp(1, 5) as usize;
    let strip = |offset: usize| {
        Line::from(
            (0..8)
                .map(|i| {
                    let style = Style::new().bg(rgb(t.palette[offset + i]));
                    Span::styled(" ".repeat(swatch_width), style)
                })
                .collect::<Vec<_>>(),
        )
    };
    if compact {
        f.render_widget(Paragraph::new(vec![strip(0), strip(8)]).block(block), area);
        return;
    }

    let inner = block.inner(area);
    if inner.width >= SAMPLE_WIDTH + GRID_WIDTH + 4 {
        // Wide: the colors as a grid to the right of the sample.
        f.render_widget(block, area);
        let [sample, _, grid] = Layout::horizontal([
            Constraint::Min(SAMPLE_WIDTH),
            Constraint::Length(2),
            Constraint::Length(GRID_WIDTH),
        ])
        .areas(inner);
        f.render_widget(Paragraph::new(sample_lines(t)), sample);
        draw_palette_grid(f, t, grid);
    } else {
        let mut lines = sample_lines(t);
        lines.push(strip(0));
        lines.push(strip(8));
        f.render_widget(Paragraph::new(lines).block(block), area);
    }
}

/// A few lines of a shell session in the theme's colors.
fn sample_lines(t: &Theme) -> Vec<Line<'static>> {
    let bg = rgb(t.background);
    let fg = Style::new().fg(rgb(t.foreground));
    let c = |i: usize| Style::new().fg(rgb(t.palette[i])).bg(bg);
    vec![
        Line::from(vec![
            Span::styled("~/code/omarchy", c(4).bold()),
            Span::styled(" on ", fg),
            Span::styled(" main ", c(5)),
            Span::styled("!2", c(3)),
        ]),
        Line::from(vec![Span::styled("❯ ", c(2)), Span::styled("ls", fg)]),
        Line::from(vec![
            Span::styled("bin  config  ", c(4).bold()),
            Span::styled("install.sh  ", c(2)),
            Span::styled("README.md  ", fg),
            Span::styled("logo.svg  ", c(6)),
            Span::styled("backup.tar.gz", c(1).bold()),
        ]),
        Line::from(vec![
            Span::styled("❯ ", c(2)),
            Span::styled("git log --oneline -2", fg),
        ]),
        Line::from(vec![
            Span::styled("4f2a91c ", c(3)),
            Span::styled("(HEAD -> ", c(6)),
            Span::styled("main", c(2)),
            Span::styled(") Add new theme", c(6).fg(rgb(t.foreground))),
        ]),
        Line::from(vec![
            Span::styled("b81e0d7 ", c(3)),
            Span::styled("Initial commit", c(8)),
        ]),
        Line::from(vec![
            Span::styled("❯ ", c(2)),
            Span::styled(" ", Style::new().bg(rgb(t.cursor))),
        ]),
    ]
}

/// The 16 colors as two labelled rows of swatches, centered in `area`.
fn draw_palette_grid(f: &mut Frame, t: &Theme, area: Rect) {
    let row = |offset: usize| {
        Line::from(
            (0..8)
                .flat_map(|i| {
                    let style = Style::new().bg(rgb(t.palette[offset + i]));
                    [Span::raw(" "), Span::styled("    ", style)]
                })
                .skip(1)
                .collect::<Vec<_>>(),
        )
    };
    let label = |text: &'static str| Line::from(text).style(Style::new().fg(rgb(t.palette[8])));
    let lines = vec![
        label("normal"),
        row(0),
        row(0),
        Line::raw(""),
        label("bright"),
        row(8),
        row(8),
    ];
    let top = area.y + area.height.saturating_sub(lines.len() as u16) / 2;
    f.render_widget(Paragraph::new(lines), Rect { y: top, ..area });
}
