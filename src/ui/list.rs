//! The theme list, with markers, swatches and a legend.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Padding, Paragraph};

use super::{panel, rgb};
use crate::app::{App, Group, ModeFilter};

pub(super) fn draw_themes(f: &mut Frame, app: &mut App, area: Rect) {
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
    let inner = block.inner(area);
    let list = List::new(items).block(block).scroll_padding(2);
    f.render_stateful_widget(list, area, &mut app.list);
    // A key to the markers, when the list leaves room for it.
    if inner.height as usize >= app.order.len() + 4 && inner.width >= 30 {
        let legend = vec![
            Line::from(vec![
                Span::raw("●").green(),
                Span::raw(" applied   "),
                Span::raw("★").yellow(),
                Span::raw(" favorite   "),
                Span::raw("·"),
                Span::raw(" recent"),
            ]),
            Line::from(vec![
                Span::raw("☀").yellow(),
                Span::raw(" light theme   "),
                Span::raw("☾").yellow(),
                Span::raw(" dark theme"),
            ]),
        ];
        let rect = Rect {
            y: inner.y + inner.height - 2,
            height: 2,
            ..inner
        };
        f.render_widget(Paragraph::new(legend).dark_gray(), rect);
    }
}
