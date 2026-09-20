//! The main pane: the full aggregated timeline for the selected story —
//! article snippets, machine alerts and videos interleaved in time order.

use crate::app::{ago, App, Pane};
use crate::model::{Item, ItemKind};
use crate::thumb::ThumbState;
use chrono::{DateTime, Datelike, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState};

pub fn render(f: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    let focused = app.focus == Pane::Timeline;
    let border = Style::default().fg(if focused {
        Color::Rgb(120, 200, 255)
    } else {
        Color::Rgb(70, 78, 90)
    });

    let Some(story) = app.selected_story.clone().and_then(|id| app.story(&id).cloned()) else {
        app.timeline_block_starts.clear();
        let hint = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                "   Select a story on the left, or click a marker on the map.",
                Style::default().fg(Color::Rgb(120, 130, 145)),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "   Its full timeline — wire copy, alerts and video — appears here.",
                Style::default().fg(Color::Rgb(90, 100, 115)),
            )),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .border_style(border)
                .title(Span::styled(
                    " TIMELINE ",
                    Style::default().fg(Color::Rgb(180, 210, 235)).add_modifier(Modifier::BOLD),
                )),
        );
        f.render_widget(hint, area);
        return;
    };

    let items = app.timeline();
    let cursor = app.timeline_cursor.min(items.len().saturating_sub(1));
    let width = area.width.saturating_sub(4) as usize;

    // Build the rendered lines, remembering which item each block starts at so
    // the cursor can be scrolled to.
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut block_starts: Vec<usize> = Vec::new();
    let mut last_day: Option<u32> = None;

    for (idx, item) in items.iter().enumerate() {
        // Day separators keep a multi-day timeline legible.
        let day = item.published.day();
        if last_day != Some(day) {
            if last_day.is_some() {
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!(" ── {} ", item.published.format("%A %e %B %Y")),
                Style::default().fg(Color::Rgb(85, 95, 110)).add_modifier(Modifier::DIM),
            )));
            last_day = Some(day);
        }

        block_starts.push(lines.len());
        // The picture tracks the cursor regardless of which pane has
        // keyboard focus — clicking an entry (which focuses the timeline
        // anyway) or arrowing onto it both count as "looking at this one".
        let is_cursor = idx == cursor;
        let selected = is_cursor && focused;
        let thumb = if is_cursor {
            item.thumbnail.as_ref().and_then(|u| app.thumbnails.get(u))
        } else {
            None
        };
        lines.extend(render_item(item, now, width, selected, thumb));
    }

    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No entries match the current type filter.",
            Style::default().fg(Color::Rgb(120, 130, 145)),
        )));
    }

    // Everything needed from the borrowed items has been rendered into owned
    // lines, so the borrow can end before the scroll position is written back.
    let item_count = items.len();
    drop(items);

    // Published so a click on the rendered pane can be mapped back to the
    // item whose block contains that line.
    app.timeline_block_starts = block_starts.clone();

    // Scroll so the cursor's block is on screen.
    let view_h = area.height.saturating_sub(2) as usize;
    let cursor_line = block_starts.get(cursor).copied().unwrap_or(0);
    let max_scroll = lines.len().saturating_sub(view_h);
    let scroll = if cursor_line < app.timeline_scroll {
        cursor_line
    } else if cursor_line + 4 > app.timeline_scroll + view_h {
        cursor_line.saturating_sub(view_h.saturating_sub(5))
    } else {
        app.timeline_scroll
    }
    .min(max_scroll);
    app.timeline_scroll = scroll;

    let kind_note = match app.kind_filter {
        Some(k) => format!("  [only {}]", kind_label(k)),
        None => String::new(),
    };
    let title = format!(
        " TIMELINE · {} · {} entries · {} sources{} ",
        truncate(&story.title, 46),
        item_count,
        story.sources().len(),
        kind_note
    );

    let para = Paragraph::new(lines.clone())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(if focused { BorderType::Thick } else { BorderType::Plain })
                .border_style(border)
                .title(Span::styled(
                    title,
                    Style::default()
                        .fg(story.category.color())
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .scroll((scroll as u16, 0));

    f.render_widget(para, area);

    if lines.len() > view_h {
        let mut sb = ScrollbarState::new(lines.len()).position(scroll);
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

/// Which timeline entry a click landed on, using the block offsets published
/// by the last render. `None` when nothing is selected or the pane wasn't
/// where the click was rendered.
pub fn row_at(app: &App, area: Rect, row: u16) -> Option<usize> {
    let starts = &app.timeline_block_starts;
    if starts.is_empty() || row <= area.y || row >= area.bottom().saturating_sub(1) {
        return None;
    }
    let line = app.timeline_scroll + (row - area.y - 1) as usize;
    // The entry whose block contains this line: the last block that starts
    // at or before it. A click on the day separator above the first block
    // still resolves to that first entry, which is a harmless rough edge.
    let idx = match starts.binary_search(&line) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    Some(idx.min(starts.len() - 1))
}

/// One timeline entry: a header line and a wrapped snippet.
fn render_item(
    item: &Item,
    now: DateTime<Utc>,
    width: usize,
    selected: bool,
    thumb: Option<&ThumbState>,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let accent = match item.kind {
        ItemKind::Video => Color::Rgb(255, 120, 120),
        ItemKind::Alert => item.severity.color(),
        ItemKind::Report => Color::Rgb(150, 200, 255),
        ItemKind::Article => Color::Rgb(190, 200, 212),
    };
    let bar = if selected { "┃" } else { "│" };

    let mut head = vec![
        Span::styled(
            format!("{bar} "),
            Style::default().fg(if selected { Color::Rgb(255, 220, 130) } else { Color::Rgb(60, 68, 80) }),
        ),
        Span::styled(
            format!("{:>5} ", ago(now, item.published)),
            Style::default().fg(Color::Rgb(115, 125, 140)),
        ),
        Span::styled(
            format!("{} ", item.kind.glyph()),
            Style::default().fg(accent),
        ),
        Span::styled(
            format!("{} ", truncate(&item.source, 22)),
            Style::default().fg(Color::Rgb(125, 150, 175)),
        ),
    ];
    if item.severity >= crate::model::Severity::Severe {
        head.push(Span::styled(
            format!("[{}] ", item.severity.label()),
            Style::default().fg(item.severity.color()).add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(lang) = &item.language {
        head.push(Span::styled(
            format!("{lang} "),
            Style::default().fg(Color::Rgb(210, 150, 220)).add_modifier(Modifier::BOLD),
        ));
    }
    out.push(Line::from(head));

    // Headline.
    let title_style = Style::default()
        .fg(if selected { Color::Rgb(255, 255, 255) } else { Color::Rgb(222, 228, 238) })
        .add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() });
    for (i, chunk) in wrap(&item.title, width.saturating_sub(10)).into_iter().enumerate() {
        out.push(Line::from(vec![
            Span::styled(
                format!("{bar}       "),
                Style::default().fg(if selected { Color::Rgb(255, 220, 130) } else { Color::Rgb(60, 68, 80) }),
            ),
            Span::styled(chunk, if i == 0 { title_style } else { title_style.remove_modifier(Modifier::BOLD) }),
        ]));
    }

    // Snippet.
    if !item.snippet.trim().is_empty() {
        for chunk in wrap(&item.snippet, width.saturating_sub(10)).into_iter().take(3) {
            out.push(Line::from(vec![
                Span::styled(
                    format!("{bar}       "),
                    Style::default().fg(Color::Rgb(50, 56, 66)),
                ),
                Span::styled(chunk, Style::default().fg(Color::Rgb(146, 156, 170))),
            ]));
        }
    }

    // Facts: magnitude, alert level, casualty figures, place.
    let mut facts: Vec<String> = Vec::new();
    if let Some(p) = &item.place {
        facts.push(format!("◎ {}", p.name));
    }
    for (k, v) in &item.facts {
        if k == "anchor" || k == "post" {
            continue;
        }
        facts.push(format!("{k}: {v}"));
    }
    if !facts.is_empty() {
        out.push(Line::from(vec![
            Span::styled(
                format!("{bar}       "),
                Style::default().fg(Color::Rgb(50, 56, 66)),
            ),
            Span::styled(
                truncate(&facts.join("  ·  "), width.saturating_sub(10)),
                Style::default().fg(Color::Rgb(120, 140, 120)),
            ),
        ]));
    }

    // A picture, but only for the entry the cursor is on — that is also the
    // only one ever fetched, so nothing else could show one anyway.
    match thumb {
        Some(ThumbState::Ready(raster)) => {
            for row in crate::ui::thumb::rows(raster, width.saturating_sub(10)) {
                let mut spans = vec![Span::styled(
                    format!("{bar}       "),
                    Style::default().fg(Color::Rgb(60, 68, 80)),
                )];
                spans.extend(row);
                out.push(Line::from(spans));
            }
        }
        Some(ThumbState::Loading) => {
            out.push(Line::from(vec![
                Span::styled(format!("{bar}       "), Style::default().fg(Color::Rgb(50, 56, 66))),
                Span::styled("loading preview…", Style::default().fg(Color::Rgb(100, 110, 125))),
            ]));
        }
        Some(ThumbState::Failed) | None => {}
    }

    out.push(Line::from(""));
    out
}

fn kind_label(k: ItemKind) -> &'static str {
    match k {
        ItemKind::Article => "articles",
        ItemKind::Alert => "alerts",
        ItemKind::Video => "video",
        ItemKind::Report => "reports",
    }
}

/// Greedy word wrap. `Paragraph`'s own wrapping cannot be used here because
/// each line carries its own gutter styling.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    if width < 8 {
        return vec![text.chars().take(width).collect()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let wlen = word.chars().count();
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + wlen <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
        // A single word longer than the pane must still be broken.
        while current.chars().count() > width {
            let head: String = current.chars().take(width).collect();
            let tail: String = current.chars().skip(width).collect();
            lines.push(head);
            current = tail;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Wrap is used by the detail overlay too.
pub fn wrap_for(text: &str, width: usize) -> Vec<String> {
    wrap(text, width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_word_boundaries() {
        let got = wrap("the quick brown fox jumps", 11);
        assert_eq!(got, vec!["the quick", "brown fox", "jumps"]);
    }

    #[test]
    fn breaks_overlong_words() {
        let got = wrap("supercalifragilistic", 8);
        assert!(got.iter().all(|l| l.chars().count() <= 8));
        assert_eq!(got.concat(), "supercalifragilistic");
    }

    #[test]
    fn empty_text_yields_one_empty_line() {
        assert_eq!(wrap("", 20), vec![String::new()]);
    }
}
