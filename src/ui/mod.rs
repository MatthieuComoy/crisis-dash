//! Screen layout: header strip, three panes, footer, and the modal overlays.

pub mod map;
pub mod stories;
pub mod thumb;
pub mod timeline;

use crate::app::{ago, App, Overlay, Pane};
use crate::model::Severity;
use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Row, Table, Wrap};

/// Panes, so mouse events can be routed to whatever was clicked.
pub struct Layout {
    pub stories: Rect,
    pub map: Rect,
    pub timeline: Rect,
}

pub fn compute_layout(area: Rect) -> Layout {
    use ratatui::layout::{Constraint, Direction, Layout as L};
    let rows = L::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(10),   // body
            Constraint::Length(1), // footer
        ])
        .split(area);

    // On a narrow terminal the list would crush the map, so give the list a
    // fixed sensible width rather than a percentage.
    let list_width = (area.width / 3).clamp(30, 52);
    let cols = L::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(list_width), Constraint::Min(30)])
        .split(rows[1]);

    // The map gets the top slice; the timeline is the main working area.
    let map_height = (cols[1].height as f32 * 0.46) as u16;
    let right = L::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(map_height.clamp(8, cols[1].height.saturating_sub(6))),
            Constraint::Min(6),
        ])
        .split(cols[1]);

    Layout { stories: cols[0], map: right[0], timeline: right[1] }
}

pub fn draw(f: &mut Frame, app: &mut App, now: DateTime<Utc>) {
    let area = f.area();
    if area.width < 60 || area.height < 18 {
        let msg = Paragraph::new("Terminal too small — needs at least 60x18.")
            .style(Style::default().fg(Color::Rgb(255, 140, 140)));
        f.render_widget(msg, area);
        return;
    }

    let l = compute_layout(area);
    let rows = ratatui::layout::Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(10), Constraint::Length(1)])
        .split(area);

    header(f, rows[0], app, now);
    stories::render(f, l.stories, app, now);
    map::render(f, l.map, app, now);
    timeline::render(f, l.timeline, app, now);
    footer(f, rows[2], app, now);

    match app.overlay {
        Overlay::Sources => sources_overlay(f, area, app, now),
        Overlay::Help => help_overlay(f, area),
        Overlay::Item => item_overlay(f, area, app, now),
        Overlay::None => {}
    }
}

fn header(f: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    let (ok, total) = app.healthy_sources();
    let worst = app.worst_severity(now);
    let visible = app.visible(now);
    let active = visible.len();
    let critical = visible.iter().filter(|s| s.severity >= Severity::Severe).count();
    drop(visible);

    let status_color = if ok == 0 && total > 0 {
        Color::Rgb(255, 90, 90)
    } else if ok * 4 < total * 3 {
        Color::Rgb(240, 190, 80)
    } else {
        Color::Rgb(120, 220, 140)
    };

    let last = app
        .last_item_at
        .map(|t| format!("{} ago", ago(now, t)))
        .unwrap_or_else(|| "—".into());

    let mut line1 = vec![
        Span::styled(
            " CRISIS DASH ",
            Style::default()
                .fg(Color::Rgb(15, 20, 28))
                .bg(worst.color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{active} active"),
            Style::default().fg(Color::Rgb(225, 232, 240)).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(Color::Rgb(70, 78, 90))),
        Span::styled(
            format!("{critical} severe+"),
            Style::default().fg(Severity::Severe.color()),
        ),
        Span::styled("  ·  ", Style::default().fg(Color::Rgb(70, 78, 90))),
        Span::styled(
            format!("{} items", app.clusterer.item_count()),
            Style::default().fg(Color::Rgb(160, 172, 186)),
        ),
        Span::styled("  ·  ", Style::default().fg(Color::Rgb(70, 78, 90))),
        Span::styled(
            format!("sources {ok}/{total}"),
            Style::default().fg(status_color),
        ),
        Span::styled("  ·  ", Style::default().fg(Color::Rgb(70, 78, 90))),
        Span::styled(
            format!("last {last}"),
            Style::default().fg(Color::Rgb(160, 172, 186)),
        ),
        Span::styled("  ·  ", Style::default().fg(Color::Rgb(70, 78, 90))),
        Span::styled(
            format!("sort {}", app.sort.label()),
            Style::default().fg(Color::Rgb(150, 165, 185)),
        ),
    ];
    // Several of these can be true at once (paused *and* not following top,
    // say), so each gets its own optional fragment rather than one slot that
    // can only show a single state.
    if app.paused {
        line1.push(Span::styled(
            "  ·  PAUSED",
            Style::default().fg(Color::Rgb(255, 180, 80)).add_modifier(Modifier::BOLD),
        ));
    }
    if app.follow_top {
        line1.push(Span::styled(
            "  ·  FOLLOWING TOP",
            Style::default().fg(Color::Rgb(120, 220, 140)),
        ));
    }
    if !app.alerts_enabled {
        line1.push(Span::styled(
            "  ·  ALERTS OFF",
            Style::default().fg(Color::Rgb(140, 150, 165)),
        ));
    }
    line1.push(Span::styled(
        format!("   {} UTC", now.format("%H:%M:%S")),
        Style::default().fg(Color::Rgb(110, 120, 135)),
    ));
    let line1 = Line::from(line1);

    // Category strip doubles as a legend for the map colours.
    let counts = app.category_counts(now);
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    for (cat, n) in counts.iter().take(9) {
        let selected = app.filter == Some(*cat);
        spans.push(Span::styled(
            format!(" {} {} ", cat.glyph(), cat.label()),
            Style::default()
                .fg(if selected { Color::Rgb(20, 24, 30) } else { cat.color() })
                .bg(if selected { cat.color() } else { Color::Reset })
                .add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }),
        ));
        spans.push(Span::styled(
            format!("{n}"),
            Style::default().fg(Color::Rgb(150, 160, 175)),
        ));
        spans.push(Span::styled(" ", Style::default()));
    }

    let block = Block::default()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(Color::Rgb(55, 62, 74)));
    let para = Paragraph::new(vec![line1, Line::from(spans)]).block(block);
    f.render_widget(para, area);
}

fn footer(f: &mut Frame, area: Rect, app: &App, now: DateTime<Utc>) {
    if app.searching {
        let p = Paragraph::new(Line::from(vec![
            Span::styled(
                " search: ",
                Style::default().fg(Color::Rgb(20, 24, 30)).bg(Color::Rgb(120, 200, 255)),
            ),
            Span::styled(
                format!(" {}", app.search),
                Style::default().fg(Color::Rgb(255, 255, 255)).add_modifier(Modifier::BOLD),
            ),
            Span::styled("▏", Style::default().fg(Color::Rgb(120, 200, 255))),
            Span::styled(
                "   enter: apply   esc: cancel",
                Style::default().fg(Color::Rgb(110, 120, 135)),
            ),
        ]));
        f.render_widget(p, area);
        return;
    }

    // The newest headline anywhere, as a live ticker.
    let ticker = app
        .newest_items(now, 1)
        .first()
        .map(|i| format!("{} · {}", i.source, i.title))
        .unwrap_or_else(|| "waiting for first items…".into());

    let keys = "↑↓ story  ⇥ pane  enter open  z zoom  r region  m marker  c cat  x sev  w zone  v type  t sort  a alerts  s sources  / search  ? help  q quit";
    let width = area.width as usize;
    let keys_len = keys.chars().count();
    let ticker_room = width.saturating_sub(keys_len + 4);

    let p = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" ▸ {} ", truncate(&ticker, ticker_room.max(10))),
            Style::default().fg(Color::Rgb(140, 190, 230)),
        ),
        Span::styled(
            format!("{:>w$}", keys, w = width.saturating_sub(ticker_room.max(10) + 4)),
            Style::default().fg(Color::Rgb(95, 105, 120)),
        ),
    ]));
    f.render_widget(p, area);
}

// ---------------------------------------------------------------- overlays

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let w = area.width * pct_x / 100;
    let h = area.height * pct_y / 100;
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn sources_overlay(f: &mut Frame, area: Rect, app: &App, now: DateTime<Utc>) {
    let rect = centered(area, 86, 82);
    f.render_widget(Clear, rect);

    let mut rows: Vec<Row> = Vec::new();
    for s in app.statuses.values() {
        let (state, color) = match (&s.last_ok, &s.last_error) {
            (_, Some(e)) => (e.clone(), Color::Rgb(255, 120, 110)),
            (Some(_), None) => ("ok".to_string(), Color::Rgb(120, 220, 140)),
            (None, None) => ("starting".to_string(), Color::Rgb(160, 170, 185)),
        };
        rows.push(Row::new(vec![
            Span::styled(s.name.clone(), Style::default().fg(Color::Rgb(215, 222, 232))),
            Span::styled(s.kind.label().to_string(), Style::default().fg(Color::Rgb(140, 160, 185))),
            Span::styled(state, Style::default().fg(color)),
            Span::styled(
                s.last_ok.map(|t| ago(now, t)).unwrap_or_else(|| "—".into()),
                Style::default().fg(Color::Rgb(150, 160, 175)),
            ),
            Span::styled(format!("{}", s.last_items), Style::default().fg(Color::Rgb(150, 160, 175))),
            Span::styled(format!("{}", s.items_total), Style::default().fg(Color::Rgb(150, 160, 175))),
            Span::styled(
                format!("{}/{}", s.failures, s.fetches),
                Style::default().fg(if s.failures > 0 {
                    Color::Rgb(230, 180, 90)
                } else {
                    Color::Rgb(120, 130, 145)
                }),
            ),
            Span::styled(format!("{}ms", s.latency_ms), Style::default().fg(Color::Rgb(120, 130, 145))),
        ]));
    }

    let kinds = app.source_kind_counts();
    let mut summary: Vec<String> = kinds
        .iter()
        .map(|(k, (ok, total))| format!("{} {}/{}", k.label(), ok, total))
        .collect();
    summary.push(format!("up {}", ago(now, app.started)));
    summary.push(format!("{} kept / {} seen", app.items_kept, app.items_seen));
    summary.push(format!("{} stories tracked", app.clusterer.story_count()));

    let table = Table::new(
        rows,
        [
            Constraint::Min(24),
            Constraint::Length(13),
            Constraint::Min(16),
            Constraint::Length(7),
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(9),
            Constraint::Length(8),
        ],
    )
    .header(
        Row::new(vec!["SOURCE", "KIND", "STATE", "AGE", "NEW", "TOTAL", "FAIL/REQ", "LATENCY"])
            .style(Style::default().fg(Color::Rgb(130, 165, 200)).add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(Color::Rgb(120, 200, 255)))
            .title(Span::styled(
                format!(" SOURCES · {} · esc to close ", summary.join("  ")),
                Style::default().fg(Color::Rgb(190, 220, 245)).add_modifier(Modifier::BOLD),
            )),
    );
    f.render_widget(table, rect);
}

fn item_overlay(f: &mut Frame, area: Rect, app: &App, now: DateTime<Utc>) {
    let rect = centered(area, 78, 70);
    f.render_widget(Clear, rect);
    let Some(item) = app.current_item() else { return };
    let width = rect.width.saturating_sub(4) as usize;

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        item.title.clone(),
        Style::default().fg(Color::Rgb(255, 255, 255)).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} {}", item.kind.glyph(), item.source),
            Style::default().fg(Color::Rgb(140, 175, 210)),
        ),
        Span::styled(
            format!("   {} ago   ", ago(now, item.published)),
            Style::default().fg(Color::Rgb(130, 140, 155)),
        ),
        Span::styled(
            format!("[{}] ", item.severity.label()),
            Style::default().fg(item.severity.color()).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            item.category.label().to_string(),
            Style::default().fg(item.category.color()),
        ),
    ]));
    lines.push(Line::from(""));
    for l in timeline::wrap_for(&item.snippet, width) {
        lines.push(Line::from(Span::styled(l, Style::default().fg(Color::Rgb(200, 208, 220)))));
    }
    lines.push(Line::from(""));
    if let Some(p) = &item.place {
        lines.push(Line::from(Span::styled(
            format!("◎ {}   {:.3}, {:.3}", p.name, p.point.lat, p.point.lon),
            Style::default().fg(Color::Rgb(140, 200, 150)),
        )));
    }
    for (k, v) in &item.facts {
        lines.push(Line::from(Span::styled(
            format!("{k}: {v}"),
            Style::default().fg(Color::Rgb(150, 170, 150)),
        )));
    }

    let thumb = item.thumbnail.as_ref().and_then(|u| app.thumbnails.get(u));
    match thumb {
        Some(crate::thumb::ThumbState::Ready(raster)) => {
            lines.push(Line::from(""));
            for row in thumb::rows(raster, width) {
                lines.push(Line::from(row));
            }
        }
        Some(crate::thumb::ThumbState::Loading) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                "loading preview…",
                Style::default().fg(Color::Rgb(110, 120, 135)),
            )));
        }
        Some(crate::thumb::ThumbState::Failed) | None => {}
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        item.url.clone(),
        Style::default().fg(Color::Rgb(110, 170, 230)).add_modifier(Modifier::UNDERLINED),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "enter / o: open in browser    y: copy url to clipboard    esc: close",
        Style::default().fg(Color::Rgb(110, 120, 135)),
    )));

    let p = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Thick)
                .border_style(Style::default().fg(item.category.color()))
                .title(Span::styled(
                    " ENTRY ",
                    Style::default().fg(Color::Rgb(200, 220, 240)).add_modifier(Modifier::BOLD),
                )),
        );
    f.render_widget(p, rect);
}

const HELP: &[(&str, &str)] = &[
    ("↑ ↓ / j k", "move through the story list"),
    ("PgUp PgDn", "jump ten stories"),
    ("Tab / ← →", "switch pane (stories · map · timeline)"),
    ("J K", "scroll the timeline of the selected story"),
    ("Enter", "open the entry overlay for the timeline cursor"),
    ("o", "open the entry's URL in your browser"),
    ("y", "copy the entry's URL to the clipboard"),
    ("", ""),
    ("click a story", "select it, and centre the map on it"),
    ("click the map", "select the nearest story marker"),
    ("click the timeline", "open that entry's full detail"),
    ("scroll wheel", "scroll whichever pane is under the pointer"),
    ("", ""),
    ("f", "follow the top-ranked story as the ranking changes"),
    ("z", "toggle auto-zoom to the selected story"),
    ("r / R", "cycle map region forward / back"),
    ("m", "cycle map glyphs (braille · dot · block · half) — try this if"),
    ("", "  braille looks illegible in your terminal font"),
    ("c / C", "cycle category filter forward / back"),
    ("x / X", "cycle minimum severity filter (watch·elevated·severe·critical)"),
    ("w / W", "cycle zone filter (Europe·Middle East·Africa·Asia·Americas·Oceania)"),
    ("v", "cycle entry-type filter (video, alert, article…)"),
    ("t", "cycle sort (latest · heat · severity)"),
    ("/", "search stories; Esc clears search, then filters"),
    ("", ""),
    ("s", "source health panel"),
    ("a", "toggle alerts: bell + desktop notification + sound, and switches"),
    ("", "  to the story, when elevated+ hits Europe or critical+ hits anywhere"),
    ("space", "pause / resume ingestion"),
    ("F5 or g", "force every source to refresh now"),
    ("?", "this help"),
    ("Esc", "clear search, then filters, then re-follow the top story"),
    ("q / Ctrl-C", "quit"),
];

fn help_overlay(f: &mut Frame, area: Rect) {
    let rect = centered(area, 62, 84);
    f.render_widget(Clear, rect);
    let mut lines: Vec<Line> = Vec::new();
    for (key, desc) in HELP {
        if key.is_empty() {
            lines.push(Line::from(""));
            continue;
        }
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {key:<14}"),
                Style::default().fg(Color::Rgb(255, 215, 120)).add_modifier(Modifier::BOLD),
            ),
            Span::styled(desc.to_string(), Style::default().fg(Color::Rgb(205, 213, 224))),
        ]));
    }
    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Thick)
            .border_style(Style::default().fg(Color::Rgb(120, 200, 255)))
            .title(Span::styled(
                " KEYS · esc to close ",
                Style::default().fg(Color::Rgb(190, 220, 245)).add_modifier(Modifier::BOLD),
            )),
    );
    f.render_widget(p, rect);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{cut}…")
}

/// Which pane a point falls in, for mouse routing.
pub fn pane_at(layout: &Layout, col: u16, row: u16) -> Option<Pane> {
    let hit = |r: Rect| col >= r.x && col < r.right() && row >= r.y && row < r.bottom();
    if hit(layout.stories) {
        Some(Pane::Stories)
    } else if hit(layout.map) {
        Some(Pane::Map)
    } else if hit(layout.timeline) {
        Some(Pane::Timeline)
    } else {
        None
    }
}
