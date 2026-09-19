//! The left-hand list of currently active stories.

use crate::app::{ago, App, Pane};
use crate::ui::map::TitleShort;
use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Scrollbar,
    ScrollbarOrientation, ScrollbarState};

pub fn render(f: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    let focused = app.focus == Pane::Stories;
    let visible = app.visible(now);
    let selected_idx = app.selected_index(&visible);
    let total = visible.len();

    let inner_width = area.width.saturating_sub(4) as usize;
    let rows: Vec<ListItem<'static>> = visible
        .iter()
        .map(|story| {
            let cat = story.category;
            let heat = story.heat(now);
            let is_selected = Some(story.id.as_str()) == app.selected_story.as_deref();

            // Row 1: severity pip, category tag, title.
            let title_width = inner_width.saturating_sub(10).max(10);
            let mut line1 = vec![
                Span::styled(
                    format!("{} ", heat_glyph(heat)),
                    Style::default().fg(story.severity.color()),
                ),
                Span::styled(
                    format!("{} ", cat.tag()),
                    Style::default().fg(cat.color()).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    story.title_short(title_width),
                    Style::default()
                        .fg(if is_selected {
                            Color::Rgb(255, 255, 255)
                        } else {
                            Color::Rgb(205, 212, 222)
                        })
                        .add_modifier(if is_selected { Modifier::BOLD } else { Modifier::empty() }),
                ),
            ];
            if story.unread > 0 {
                line1.push(Span::styled(
                    format!(" +{}", story.unread),
                    Style::default().fg(Color::Rgb(120, 230, 150)).add_modifier(Modifier::BOLD),
                ));
            }

            // Row 2: where, how many reports, how many sources, how fresh.
            let place = story
                .focus()
                .map(|p| p.name)
                .unwrap_or_else(|| "unlocated".into());
            let videos = story.video_count();
            let mut meta = format!(
                "    {} · {} reports · {} src · {}",
                truncate(&place, 18),
                story.items.len(),
                story.sources().len(),
                ago(now, story.last_update)
            );
            if videos > 0 {
                meta.push_str(&format!(" · {videos}▶"));
            }
            let line2 = Line::from(Span::styled(
                truncate(&meta, inner_width),
                Style::default().fg(Color::Rgb(118, 128, 142)),
            ));

            ListItem::new(vec![Line::from(line1), line2])
        })
        .collect();

    let mut active_filters: Vec<String> = Vec::new();
    if let Some(c) = app.filter {
        active_filters.push(c.label().to_string());
    }
    if let Some(min) = app.severity_filter {
        active_filters.push(format!("{}+", min.label()));
    }
    if let Some(z) = app.zone_filter {
        active_filters.push(z.label().to_string());
    }
    if !app.search.is_empty() {
        active_filters.push(format!("\"{}\"", app.search));
    }
    let header = if active_filters.is_empty() {
        format!(" ACTIVE STORIES ({total}) ")
    } else {
        format!(" ACTIVE STORIES · {} ({}) ", active_filters.join(" · "), total)
    };

    let list = List::new(rows)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(if focused { BorderType::Thick } else { BorderType::Plain })
                .border_style(Style::default().fg(if focused {
                    Color::Rgb(120, 200, 255)
                } else {
                    Color::Rgb(70, 78, 90)
                }))
                .title(Span::styled(
                    header,
                    Style::default()
                        .fg(Color::Rgb(180, 210, 235))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(Color::Rgb(32, 46, 62))
                .add_modifier(Modifier::BOLD),
        );

    // The borrow of `app` ends here so the scroll position can be recorded.
    drop(visible);

    let mut state = ListState::default();
    state.select(selected_idx);
    // Each row is two lines tall, so the visible window is half the pane.
    let rows_visible = (area.height.saturating_sub(2) / 2).max(1) as usize;
    let offset = selected_idx
        .map(|i| i.saturating_sub(rows_visible / 2))
        .unwrap_or(app.story_scroll)
        .min(total.saturating_sub(rows_visible.min(total)));
    *state.offset_mut() = offset;
    app.story_scroll = offset;
    app.list_origin = (area.x + 1, area.y + 1);

    f.render_stateful_widget(list, area, &mut state);

    if total > rows_visible {
        let mut sb = ScrollbarState::new(total).position(selected_idx.unwrap_or(0));
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .thumb_style(Style::default().fg(Color::Rgb(90, 110, 130))),
            area.inner(Margin { vertical: 1, horizontal: 0 }),
            &mut sb,
        );
    }
}

/// Which list row a click landed on, accounting for two-line rows and scroll.
pub fn row_at(app: &App, area: Rect, row: u16) -> Option<usize> {
    if row <= area.y || row >= area.bottom().saturating_sub(1) {
        return None;
    }
    let rel = (row - area.y - 1) as usize;
    Some(app.story_scroll + rel / 2)
}

fn heat_glyph(heat: f64) -> &'static str {
    match heat {
        h if h > 14.0 => "█",
        h if h > 7.0 => "▓",
        h if h > 3.0 => "▒",
        h if h > 1.0 => "░",
        _ => "·",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}
