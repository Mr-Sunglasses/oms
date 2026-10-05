//! Drawing. The chrome uses the terminal's own ANSI colors, so it recolors along
//! with Ghostty; the preview pane paints the selected theme in true color.

use image::imageops::FilterType;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, Padding, Paragraph};
use ratatui_image::{Resize, StatefulImage};

use crate::app::{App, file_name};
use crate::repo::{Rgb, Theme};

fn rgb((r, g, b): Rgb) -> Color {
    Color::Rgb(r, g, b)
}

fn panel(title: impl Into<Line<'static>>) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::DarkGray))
        .title(title.into())
        .title_style(Style::new().fg(Color::Blue).bold())
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let [body, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(f.area());
    let [left, right] =
        Layout::horizontal([Constraint::Length(36), Constraint::Min(0)]).areas(body);
    let [preview, wallpaper] =
        Layout::vertical([Constraint::Length(12), Constraint::Min(0)]).areas(right);

    draw_themes(f, app, left);
    draw_preview(f, app.theme(), preview);
    draw_wallpaper(f, app, wallpaper);
    draw_footer(f, app, footer);
}

fn draw_themes(f: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = app
        .themes
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let marker = if app.is_applied_theme(t) {
                Span::raw("● ").green()
            } else {
                Span::raw("  ")
            };
            // Highlighted by hand so the swatches keep their colors.
            let name_style = if i == app.selected {
                Style::new().bg(Color::Blue).fg(Color::Black).bold()
            } else {
                Style::new()
            };
            let swatches: Vec<Span> = [1, 2, 3, 4, 5, 6]
                .iter()
                .map(|&i| Span::styled("█", Style::new().fg(rgb(t.palette[i]))))
                .collect();
            let name_width = width.saturating_sub(2 + 1 + 1 + swatches.len() + 1);
            let mut spans = vec![
                marker,
                Span::styled(format!(" {:<name_width$.name_width$}", t.name), name_style),
                Span::raw(" "),
            ];
            spans.extend(swatches);
            ListItem::new(Line::from(spans))
        })
        .collect();
    let count = app.themes.len();
    let list = List::new(items)
        .block(panel(format!(" Themes · {count} ")).padding(Padding::horizontal(1)))
        .scroll_padding(2);
    f.render_stateful_widget(list, area, &mut app.list);
}

fn draw_preview(f: &mut Frame, t: &Theme, area: Rect) {
    let bg = rgb(t.background);
    let fg = rgb(t.foreground);
    let c = |i: usize| Style::new().fg(rgb(t.palette[i])).bg(bg);
    let mode = if t.is_light() { "light" } else { "dark" };

    let swatch_row = |offset: usize| {
        Line::from(
            (0..8)
                .map(|i| Span::styled("     ", Style::new().bg(rgb(t.palette[offset + i]))))
                .collect::<Vec<_>>(),
        )
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("~/code/omarchy", c(4).bold()),
            Span::styled(" on ", Style::new().fg(fg)),
            Span::styled(" main ", c(5)),
            Span::styled("!2", c(3)),
        ]),
        Line::from(vec![
            Span::styled("❯ ", c(2)),
            Span::styled("ls", Style::new().fg(fg)),
        ]),
        Line::from(vec![
            Span::styled("bin  config  ", c(4).bold()),
            Span::styled("install.sh  ", c(2)),
            Span::styled("README.md  ", Style::new().fg(fg)),
            Span::styled("logo.svg  ", c(6)),
            Span::styled("backup.tar.gz", c(1).bold()),
        ]),
        Line::from(vec![
            Span::styled("❯ ", c(2)),
            Span::styled("git log --oneline -2", Style::new().fg(fg)),
        ]),
        Line::from(vec![
            Span::styled("4f2a91c ", c(3)),
            Span::styled("(HEAD -> ", c(6)),
            Span::styled("main", c(2)),
            Span::styled(") Add new theme", c(6).fg(fg)),
        ]),
        Line::from(vec![
            Span::styled("b81e0d7 ", c(3)),
            Span::styled("Initial commit", c(8)),
        ]),
        Line::from(vec![
            Span::styled("❯ ", c(2)),
            Span::styled(" ", Style::new().bg(rgb(t.cursor))),
        ]),
        swatch_row(0),
        swatch_row(8),
    ];
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
        .style(Style::new().bg(bg).fg(fg));
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_wallpaper(f: &mut Frame, app: &mut App, area: Rect) {
    let theme = app.theme();
    let count = theme.wallpapers.len();
    let index = app.wallpaper[app.selected];
    let path = app.wallpaper_path().cloned();

    let mut title = vec![Span::raw(" Wallpaper ")];
    if let Some(path) = &path {
        title.push(Span::raw(format!(
            "{}/{count} · {} ",
            index + 1,
            file_name(path)
        )));
        if app.is_applied_wallpaper(path) {
            title.push(Span::raw("● ").green());
        }
    }
    let block =
        panel(Line::from(title)).title_bottom(Line::from(" ← → browse ").centered().dark_gray());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(path) = path else {
        f.render_widget(centered(format!("{} has no wallpapers", theme.name)), inner);
        return;
    };
    let size = app.loader.get(&path).map(|i| (i.width(), i.height()));
    match (&mut app.image, size) {
        (Some((shown, protocol)), Some(size)) if *shown == path => {
            let resize = Resize::Scale(Some(FilterType::Triangle));
            f.render_stateful_widget(
                StatefulImage::default().resize(resize),
                center_image(inner, size, app.cell_aspect),
                protocol,
            );
        }
        _ => {
            let message = match app.loader.errors.get(&path) {
                Some(e) => format!("Can't preview {}: {e}", file_name(&path)),
                None => "Loading…".to_string(),
            };
            f.render_widget(centered(message), inner);
        }
    }
}

/// The largest rect with the picture's aspect ratio, centered in `area`.
fn center_image(area: Rect, (w, h): (u32, u32), cell_aspect: f64) -> Rect {
    // In cells, since cells are taller than they are wide.
    let ratio = w as f64 / h as f64 / cell_aspect;
    let (aw, ah) = (area.width as f64, area.height as f64);
    let (width, height) = if aw / ah > ratio {
        (ah * ratio, ah)
    } else {
        (aw, aw / ratio)
    };
    let (width, height) = (width.floor() as u16, height.floor() as u16);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn centered(text: String) -> Paragraph<'static> {
    Paragraph::new(vec![Line::raw(""), Line::raw(text).dark_gray()]).alignment(Alignment::Center)
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let key = |k: &'static str| Span::raw(k).blue().bold();
    let text = |t: &'static str| Span::raw(t).dark_gray();
    let live = if app.live {
        Span::raw("on").green()
    } else {
        Span::raw("off").dark_gray()
    };
    let keys = Line::from(vec![
        key(" ↑↓ "),
        text("theme  "),
        key("←→ "),
        text("wallpaper  "),
        key("enter "),
        text("apply both  "),
        key("t "),
        text("theme only  "),
        key("w "),
        text("wallpaper only  "),
        key("r "),
        text("random  "),
        key("p "),
        text("live preview "),
        live,
        text("  "),
        key("q "),
        text("quit"),
    ]);
    let status = Line::from(format!(" {}", app.status)).dark_gray();
    f.render_widget(Paragraph::new(vec![status, keys]), area);
}
