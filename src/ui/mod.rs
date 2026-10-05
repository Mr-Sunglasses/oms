//! Drawing. The chrome uses the terminal's own ANSI colors, so it recolors along
//! with Ghostty; the preview pane paints the selected theme in true color.
//!
//! This file lays the screen out; each part is drawn in its own module.

mod list;
mod overlays;
mod preview;
mod wallpaper;

use list::draw_themes;
use overlays::{draw_footer, draw_help, draw_update_prompt};
use preview::{draw_header, draw_preview};
use wallpaper::draw_wallpaper;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Paragraph};

use crate::app::App;
use crate::repo::Rgb;

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
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(f.area());
    app.thumb_rects.clear();
    draw_header(f, app, header);

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
    if let Some(version) = &app.update_prompt {
        draw_update_prompt(f, version);
    }
}

/// "2-shaded-entrance.jpg" -> "Shaded Entrance".
pub fn pretty_name(path: &std::path::Path) -> String {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let words: Vec<String> = stem
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '-' || c == '_' || c == ' ')
        .split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect();
    if words.is_empty() {
        stem.into_owned()
    } else {
        words.join(" ")
    }
}

/// The largest rect with the picture's aspect ratio, centered in `area`.
fn center_image(area: Rect, (w, h): (u32, u32), cell_aspect: f64) -> Rect {
    // In cells, since cells are taller than they are wide.
    let ratio = f64::from(w) / f64::from(h) / cell_aspect;
    let (aw, ah) = (f64::from(area.width), f64::from(area.height));
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
