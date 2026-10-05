//! Drawing. The chrome uses the terminal's own ANSI colors, so it recolors along
//! with Ghostty; the preview pane paints the selected theme in true color.

use image::imageops::FilterType;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, Padding, Paragraph};
use ratatui_image::{Resize, StatefulImage};

use crate::app::{App, Group, ModeFilter, file_name};
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

/// Below this width the picker stacks into one column.
const NARROW: u16 = 72;
/// Below this width (or a short window) the theme sample shrinks to its palette.
const MEDIUM: u16 = 112;

pub fn draw(f: &mut Frame, app: &mut App) {
    let [body, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(f.area());
    app.thumb_rects.clear();

    if body.width < NARROW {
        let list_height = (app.order.len() as u16 + 2).clamp(5, body.height * 45 / 100);
        let [list, wallpaper] =
            Layout::vertical([Constraint::Length(list_height), Constraint::Min(0)]).areas(body);
        draw_themes(f, app, list);
        draw_wallpaper(f, app, wallpaper, true);
    } else {
        let compact = body.width < MEDIUM || body.height < 30;
        let list_width = if body.width < MEDIUM { 34 } else { 40 };
        let [left, right] =
            Layout::horizontal([Constraint::Length(list_width), Constraint::Min(0)]).areas(body);
        let preview_height = if compact { 4 } else { 12 };
        let [preview, wallpaper] =
            Layout::vertical([Constraint::Length(preview_height), Constraint::Min(0)]).areas(right);
        draw_themes(f, app, left);
        draw_preview(f, app.theme(), preview, compact);
        draw_wallpaper(f, app, wallpaper, false);
    }
    draw_footer(f, app, footer);
    if app.show_help {
        draw_help(f);
    }
}

fn draw_themes(f: &mut Frame, app: &mut App, area: Rect) {
    let width = area.width.saturating_sub(4) as usize;
    let items: Vec<ListItem> = app
        .order
        .iter()
        .map(|&(i, group)| {
            let t = &app.themes[i];
            let applied = if app.is_applied_theme(t) {
                Span::raw("●").green()
            } else {
                Span::raw(" ")
            };
            let mark = match group {
                Group::Favorite => Span::raw("★").yellow(),
                Group::Recent => Span::raw("·").dark_gray(),
                Group::Other => Span::raw(" "),
            };
            // Highlighted by hand so the swatches keep their colors. Reversed
            // terminal colors stay readable whatever the theme.
            let name_style = if i == app.selected {
                Style::new().reversed().bold()
            } else {
                Style::new()
            };
            let modes = app.mode_marks(t);
            // Small squares with gaps; fewer of them when the list is narrow,
            // so names keep room.
            let colors: &[usize] = if width >= 2 + 1 + 16 + 2 + 1 + 11 {
                &[1, 2, 3, 4, 5, 6]
            } else {
                &[1, 2, 4, 5]
            };
            let swatches: Vec<Span> = colors
                .iter()
                .flat_map(|&i| {
                    [
                        Span::raw(" "),
                        Span::styled("■", Style::new().fg(rgb(t.palette[i]))),
                    ]
                })
                .skip(1)
                .collect();
            let swatch_width = colors.len() * 2 - 1;
            let name_width = width.saturating_sub(2 + 1 + 2 + swatch_width + 1);
            let mut spans = vec![
                applied,
                mark,
                Span::styled(format!(" {:<name_width$.name_width$}", t.name), name_style),
                Span::raw(format!("{modes:>2}")).yellow(),
                Span::raw(" "),
            ];
            spans.extend(swatches);
            ListItem::new(Line::from(spans))
        })
        .collect();
    let title = if app.filtering || !app.filter.is_empty() {
        let cursor = if app.filtering { "▏" } else { "" };
        format!(
            " / {}{cursor} · {} of {} ",
            app.filter,
            app.order.len(),
            app.themes.len()
        )
    } else if app.mode_filter != ModeFilter::All {
        format!(
            " Themes · {} · {} ",
            app.mode_filter.label(),
            app.order.len()
        )
    } else {
        format!(" Themes · {} ", app.themes.len())
    };
    let mut block = panel(title).padding(Padding::horizontal(1));
    app.list_inner = block.inner(area);
    if app.order.is_empty() {
        block = block.title_bottom(Line::from(" no match · esc clears ").centered().dark_gray());
    }
    let list = List::new(items).block(block).scroll_padding(2);
    f.render_stateful_widget(list, area, &mut app.list);
}

fn draw_preview(f: &mut Frame, t: &Theme, area: Rect, compact: bool) {
    let bg = rgb(t.background);
    let fg = rgb(t.foreground);
    let c = |i: usize| Style::new().fg(rgb(t.palette[i])).bg(bg);
    let mode = if t.is_light() { "light" } else { "dark" };

    // Swatches shrink to fit narrow panes.
    let swatch_width = ((area.width.saturating_sub(4)) / 8).clamp(1, 5) as usize;
    let swatch_row = |offset: usize| {
        Line::from(
            (0..8)
                .map(|i| {
                    Span::styled(
                        " ".repeat(swatch_width),
                        Style::new().bg(rgb(t.palette[offset + i])),
                    )
                })
                .collect::<Vec<_>>(),
        )
    };
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
    if compact {
        f.render_widget(
            Paragraph::new(vec![swatch_row(0), swatch_row(8)]).block(block),
            area,
        );
        return;
    }
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
    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_wallpaper(f: &mut Frame, app: &mut App, area: Rect, with_theme_name: bool) {
    app.wallpaper_area = area;
    let theme = app.theme().clone();
    let count = theme.wallpapers.len();
    let index = app.wallpaper[app.selected];
    let path = app.wallpaper_path().cloned();

    let mut title = vec![Span::raw(" ")];
    if with_theme_name {
        title.push(Span::raw(format!("{} · ", theme.name)));
    }
    title.push(Span::raw("Wallpaper "));
    if let Some(path) = &path {
        let yours = if theme.is_custom_wallpaper(index) {
            " (yours)"
        } else {
            ""
        };
        title.push(Span::raw(format!(
            "{}/{count} · {}{yours} ",
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

    // A strip of thumbnails when there's room for it.
    let mut picture = inner;
    if count > 1 && inner.height >= 16 && inner.width >= 40 {
        let strip_height = 6;
        let [main, _, strip] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(strip_height),
        ])
        .areas(inner);
        picture = main;
        draw_thumbs(f, app, &theme, strip);
    }

    let size = app
        .loader
        .get(&path)
        .map(|p| (p.image.width(), p.image.height()));
    match (&mut app.image, size) {
        (Some((shown, protocol)), Some(size)) if *shown == path => {
            let resize = Resize::Scale(Some(FilterType::Triangle));
            f.render_stateful_widget(
                StatefulImage::default().resize(resize),
                center_image(picture, size, app.cell_aspect),
                protocol,
            );
        }
        _ => {
            let message = match app.loader.errors.get(&path) {
                Some(e) => format!("Can't preview {}: {e}", file_name(&path)),
                None => "Loading…".to_string(),
            };
            f.render_widget(centered(message), picture);
        }
    }
}

/// Small pictures of all the theme's wallpapers; the selected one is outlined.
fn draw_thumbs(f: &mut Frame, app: &mut App, theme: &crate::repo::Theme, area: Rect) {
    // A 16:10 picture this many rows tall, in cells.
    let rows = area.height.saturating_sub(2).max(1);
    let cols = ((rows as f64 / app.cell_aspect) * 1.6).round() as u16;
    let width = cols + 2;
    let gap = 1;
    app.thumb_cells = (cols, rows);
    let fits = ((area.width + gap) / (width + gap)).max(1) as usize;
    let count = theme.wallpapers.len();
    let selected = app.wallpaper[app.selected];
    // Keep the selected thumbnail in view.
    let start = selected
        .saturating_sub(fits.saturating_sub(1) / 2)
        .min(count.saturating_sub(fits));
    let shown = fits.min(count);
    let total_width = shown as u16 * (width + gap) - gap;
    let mut x = area.x + area.width.saturating_sub(total_width) / 2;
    for index in start..start + shown {
        let path = &theme.wallpapers[index];
        let rect = Rect {
            x,
            y: area.y,
            width,
            height: area.height,
        };
        let is_selected = index == selected;
        let mut block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(if is_selected {
                Style::new().fg(Color::Blue).bold()
            } else {
                Style::new().fg(Color::DarkGray)
            });
        if app.is_applied_wallpaper(path) {
            block = block.title_bottom(Line::from("●").centered().green());
        }
        let inner = block.inner(rect);
        f.render_widget(block, rect);
        match app.thumbs.get_mut(path) {
            // Scale up as well as down, so the cropped picture fills the box.
            Some(protocol) => f.render_stateful_widget(
                StatefulImage::default().resize(Resize::Scale(Some(FilterType::Triangle))),
                inner,
                protocol,
            ),
            None => f.render_widget(
                Paragraph::new("…").dark_gray().alignment(Alignment::Center),
                inner,
            ),
        }
        app.thumb_rects.push((rect, index));
        x += width + gap;
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
    f.render_widget(
        Paragraph::new(Line::from(format!(" {}", app.status_line())).dark_gray()),
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

fn draw_help(f: &mut Frame) {
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
