//! The footer (status and key hints) and the popups drawn over everything.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Padding, Paragraph};

use super::panel;
use crate::app::{App, ModeFilter};

/// "oms 0.7.0 is out. Update now?" in the middle of the screen.
pub(super) fn draw_update_prompt(f: &mut Frame, version: &str) {
    let area = f.area();
    let width = 50.min(area.width.saturating_sub(4));
    let height = 7;
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height: height.min(area.height),
    };
    let lines = vec![
        Line::from(vec![
            Span::raw(format!("oms {version}")).bold(),
            Span::raw(" is out."),
        ]),
        Line::from(format!("You have {}.", env!("CARGO_PKG_VERSION"))).dark_gray(),
        Line::raw(""),
        Line::from(vec![
            Span::raw("Update now?  "),
            Span::raw("y").blue().bold(),
            Span::raw(" yes   "),
            Span::raw("n").blue().bold(),
            Span::raw(" later"),
        ]),
    ];
    let block = panel(" Update ").padding(Padding::horizontal(2));
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block), popup);
}

pub(super) fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let live = if app.live { "live on" } else { "live off" };
    let mode = format!(
        "show {}",
        match app.mode_filter {
            ModeFilter::All => "dark",
            ModeFilter::Dark => "light",
            ModeFilter::Light => "all",
        }
    );
    // In order of importance; as many as fit, always ending with help and quit.
    let hints: Vec<(&str, String)> = vec![
        ("↑↓", "theme".into()),
        ("←→", "wallpaper".into()),
        ("enter", "apply".into()),
        ("u", "undo".into()),
        ("f", "favorite".into()),
        ("/", "search".into()),
        ("tab", mode),
        ("L D", "light/dark".into()),
        ("p", live.into()),
    ];
    let tail = [("?", "help"), ("q", "quit")];
    let width_of = |k: &str, v: &str| k.chars().count() + v.chars().count() + 3;
    let mut budget =
        area.width as usize - 1 - tail.iter().map(|(k, v)| width_of(k, v)).sum::<usize>();
    let mut spans = vec![Span::raw(" ")];
    for (k, v) in &hints {
        let w = width_of(k, v);
        if w > budget {
            break;
        }
        budget -= w;
        spans.push(Span::raw(format!("{k} ")).blue().bold());
        spans.push(Span::raw(format!("{v}  ")).dark_gray());
    }
    for (k, v) in tail {
        spans.push(Span::raw(format!("{k} ")).blue().bold());
        spans.push(Span::raw(format!("{v}  ")).dark_gray());
    }

    let notice = app
        .update_available
        .as_ref()
        .map(|v| format!("oms {v} is out · press U to update "));
    let [status_area, update_area] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(notice.as_ref().map_or(0, |n| n.chars().count() as u16)),
    ])
    .areas(Rect { height: 1, ..area });
    let status = app.status_line();
    let style = if app.busy() {
        Style::new().blue()
    } else if status.starts_with('✓') || status.starts_with('↶') || status.starts_with('★') {
        Style::new().green()
    } else if status.starts_with("Error") {
        Style::new().red()
    } else {
        Style::new().dark_gray()
    };
    f.render_widget(
        Paragraph::new(Line::from(format!(" {status}")).style(style)),
        status_area,
    );
    if let Some(notice) = notice {
        f.render_widget(
            Paragraph::new(Line::from(notice).yellow().bold().right_aligned()),
            update_area,
        );
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect {
            y: area.y + 1,
            height: 1,
            ..area
        },
    );
}

const HELP: &[(&str, &str)] = &[
    ("↑ ↓  j k", "choose a theme (or scroll / click)"),
    ("← →  h l", "choose a wallpaper (or click a thumbnail)"),
    ("enter", "apply the theme and the wallpaper"),
    ("t / w", "apply only the theme / only the wallpaper"),
    ("u", "undo the last change"),
    ("r", "pick a random theme and wallpaper"),
    ("f", "favorite: ★ at the top, · marks recently used"),
    ("/", "search by name; esc clears"),
    ("tab", "show all, dark or light themes"),
    (
        "L / D",
        "use this theme (and wallpaper) in light / dark mode",
    ),
    ("p", "live preview: Ghostty follows the selection"),
    ("U", "update oms, when a new version is out"),
    ("q  esc", "quit (a live preview is undone)"),
];

const HELP_CLI: &[&str] = &[
    "oms doctor            check your setup",
    "oms rotate 30m        rotate wallpapers",
    "oms apps on nvim bat  theme other apps too",
    "oms wallpapers add    use your own pictures",
    "oms config install    Kanishk's Ghostty config",
    "oms --help            everything else",
];

pub(super) fn draw_help(f: &mut Frame) {
    let area = f.area();
    let width = 72.min(area.width.saturating_sub(4));
    let height = (HELP.len() + HELP_CLI.len() + 6) as u16;
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height: height.min(area.height),
    };
    let mut lines: Vec<Line> = HELP
        .iter()
        .map(|(k, v)| {
            Line::from(vec![
                Span::raw(format!("{k:<11}")).blue().bold(),
                Span::raw(*v),
            ])
        })
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::raw("From the command line").bold());
    lines.extend(HELP_CLI.iter().map(|l| Line::raw(*l).dark_gray()));
    let block = panel(" Keys ")
        .title_bottom(Line::from(" any key closes ").centered().dark_gray())
        .padding(Padding::new(2, 2, 1, 0));
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(lines).block(block), popup);
}
