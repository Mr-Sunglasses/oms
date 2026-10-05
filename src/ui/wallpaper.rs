//! The wallpaper preview and the strip of thumbnails under it.

use image::imageops::FilterType;
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui_image::{Resize, StatefulImage};

use super::{center_image, centered, panel, pretty_name};
use crate::app::{App, file_name};
use crate::repo::Theme;

pub(super) fn draw_wallpaper(f: &mut Frame, app: &mut App, area: Rect, with_theme_name: bool) {
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
        title.push(Span::raw(format!("{}/{count} · ", index + 1)).dark_gray());
        title.push(Span::raw(format!("{}{yours} ", pretty_name(path))));
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

    let resize = Resize::Scale(Some(FilterType::Triangle));
    let current_ready = app
        .image
        .as_ref()
        .is_some_and(|s| s.path == path && s.ready);
    if let Some(shown) = app.image.as_mut().filter(|s| s.path == path) {
        // Drawing it starts its resize on the worker; it shows once ready.
        let rect = center_image(picture, shown.size, app.cell_aspect);
        f.render_stateful_widget(
            StatefulImage::default().resize(resize.clone()),
            rect,
            &mut shown.protocol,
        );
        if !current_ready && let Some(previous) = app.previous.as_mut() {
            let rect = center_image(picture, previous.size, app.cell_aspect);
            f.render_stateful_widget(
                StatefulImage::default().resize(resize),
                rect,
                &mut previous.protocol,
            );
        }
    } else if let Some(e) = app.loader.errors.get(&path) {
        f.render_widget(
            centered(format!("Can't preview {}: {e}", file_name(&path))),
            picture,
        );
    } else if let Some(previous) = app.previous.as_mut().or(app.image.as_mut()) {
        // Still loading: keep the last picture up instead of flashing blank.
        let rect = center_image(picture, previous.size, app.cell_aspect);
        f.render_stateful_widget(
            StatefulImage::default().resize(resize),
            rect,
            &mut previous.protocol,
        );
    } else {
        f.render_widget(centered("Loading…".to_string()), picture);
    }
}

/// Small pictures of all the theme's wallpapers; the selected one is outlined.
fn draw_thumbs(f: &mut Frame, app: &mut App, theme: &Theme, area: Rect) {
    // A 16:10 picture this many rows tall, in cells.
    let rows = area.height.saturating_sub(2).max(1);
    let cols = ((f64::from(rows) / app.cell_aspect) * 1.6).round() as u16;
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
    // Arrows when there are more thumbnails than fit.
    let first = app.thumb_rects.first().map(|(r, _)| r.x);
    let middle = area.y + area.height / 2;
    if start > 0
        && let Some(x) = first
        && x >= area.x + 2
    {
        f.render_widget(
            Paragraph::new("‹").bold(),
            Rect {
                x: x - 2,
                y: middle,
                width: 1,
                height: 1,
            },
        );
    }
    if start + shown < count
        && let Some((r, _)) = app.thumb_rects.last()
        && r.right() + 2 <= area.right()
    {
        f.render_widget(
            Paragraph::new("›").bold(),
            Rect {
                x: r.right() + 1,
                y: middle,
                width: 1,
                height: 1,
            },
        );
    }
}
