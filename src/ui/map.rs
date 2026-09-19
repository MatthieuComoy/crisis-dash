//! The world map pane: coastlines, one marker per active story, and a
//! highlighted zone for the selected one.

use crate::app::{App, Pane};
use crate::geo;
use crate::model::{GeoPoint, Severity, Story};
use chrono::{DateTime, Utc};
use ratatui::prelude::*;
use ratatui::widgets::canvas::{
    Canvas, Circle, Context, Line as CanvasLine, Map, MapResolution, Points, Rectangle,
};
use ratatui::widgets::{Block, BorderType, Borders};

/// Longitude/latitude window currently shown.
#[derive(Debug, Clone, Copy)]
pub struct Viewport {
    pub lon: [f64; 2],
    pub lat: [f64; 2],
}

impl Viewport {
    fn contains(&self, p: &GeoPoint) -> bool {
        p.lon >= self.lon[0] && p.lon <= self.lon[1] && p.lat >= self.lat[0] && p.lat <= self.lat[1]
    }

    /// Terminal cell a coordinate lands on, for click hit-testing.
    fn cell(&self, p: &GeoPoint, area: Rect) -> Option<(u16, u16)> {
        if !self.contains(p) || area.width == 0 || area.height == 0 {
            return None;
        }
        let fx = (p.lon - self.lon[0]) / (self.lon[1] - self.lon[0]);
        let fy = (self.lat[1] - p.lat) / (self.lat[1] - self.lat[0]);
        let x = area.x + (fx * (area.width.saturating_sub(1)) as f64).round() as u16;
        let y = area.y + (fy * (area.height.saturating_sub(1)) as f64).round() as u16;
        Some((x.min(area.right().saturating_sub(1)), y.min(area.bottom().saturating_sub(1))))
    }
}

/// Terminal cells are about twice as tall as they are wide, so a viewport that
/// ignores that renders a world squashed flat. This expands whichever axis is
/// short until the aspect ratio matches the pane.
fn fit(center: GeoPoint, lon_span: f64, lat_span: f64, area: Rect) -> Viewport {
    let w = area.width.max(1) as f64;
    let h = area.height.max(1) as f64;
    let target = w / (2.0 * h); // desired lon_span / lat_span

    let (mut lon_span, mut lat_span) = (lon_span.max(0.5), lat_span.max(0.5));
    if lon_span / lat_span < target {
        lon_span = lat_span * target;
    } else {
        lat_span = lon_span / target;
    }

    // Keep the window on the globe.
    let lat_span = lat_span.min(170.0);
    let lon_span = lon_span.min(360.0);
    let mut lat0 = center.lat - lat_span / 2.0;
    let mut lat1 = center.lat + lat_span / 2.0;
    if lat0 < -85.0 {
        lat1 += -85.0 - lat0;
        lat0 = -85.0;
    }
    if lat1 > 85.0 {
        lat0 -= lat1 - 85.0;
        lat1 = 85.0;
    }
    let mut lon0 = center.lon - lon_span / 2.0;
    let mut lon1 = center.lon + lon_span / 2.0;
    if lon0 < -180.0 {
        lon1 += -180.0 - lon0;
        lon0 = -180.0;
    }
    if lon1 > 180.0 {
        lon0 -= lon1 - 180.0;
        lon1 = 180.0;
    }
    Viewport { lon: [lon0.max(-180.0), lon1.min(180.0)], lat: [lat0.max(-85.0), lat1.min(85.0)] }
}

/// Decide what the map should be looking at.
pub fn viewport(app: &App, area: Rect) -> (Viewport, String) {
    let selected = app
        .selected_story
        .as_ref()
        .and_then(|id| app.story(id))
        .and_then(|s| s.focus().map(|p| (s, p)));

    if app.auto_zoom {
        if let Some((story, place)) = selected {
            // Frame the affected zone with room around it. The floor matters:
            // zoomed hard onto a mid-ocean epicentre the pane is blank water.
            let span = (place.radius_deg * 6.0).clamp(24.0, 150.0);
            let vp = fit(place.point, span, span, area);
            let label = format!("{} · {}", story.title_short(28), place.name);
            return (vp, label);
        }
    }
    let (name, lon, lat) = geo::REGIONS[app.region % geo::REGIONS.len()];
    let center = GeoPoint::new((lat[0] + lat[1]) / 2.0, (lon[0] + lon[1]) / 2.0);
    let vp = fit(center, lon[1] - lon[0], lat[1] - lat[0], area);
    (vp, name.to_string())
}

pub fn render(f: &mut Frame, area: Rect, app: &mut App, now: DateTime<Utc>) {
    let focused = app.focus == Pane::Map;
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let (vp, label) = viewport(app, inner);

    // Gather what to draw. Collected up front so the paint closure borrows
    // nothing mutable from the app.
    let visible = app.visible(now);
    let selected_id = app.selected_story.clone();

    struct Marker {
        point: GeoPoint,
        color: Color,
        severity: Severity,
        selected: bool,
        label: String,
        glyph: &'static str,
        id: String,
    }

    // Only the strongest stories get a pin. Past a few dozen the map stops
    // being a map and becomes a smear.
    let mut markers: Vec<Marker> = Vec::new();
    for story in visible.iter().take(60) {
        let Some(place) = story.focus() else { continue };
        markers.push(Marker {
            point: place.point,
            color: story.category.color(),
            severity: story.severity,
            selected: Some(story.id.as_str()) == selected_id.as_deref(),
            label: story.title_short(22),
            glyph: story.category.glyph(),
            id: story.id.clone(),
        });
    }
    if let Some(id) = selected_id.as_deref() {
        if !markers.iter().any(|m| m.id == id) {
            if let Some(story) = visible.iter().find(|s| s.id == id) {
                if let Some(place) = story.focus() {
                    markers.push(Marker {
                        point: place.point,
                        color: story.category.color(),
                        severity: story.severity,
                        selected: true,
                        label: story.title_short(22),
                        glyph: story.category.glyph(),
                        id: story.id.clone(),
                    });
                }
            }
        }
    }
    // Draw the selected marker last so it sits on top.
    markers.sort_by_key(|m| m.selected);

    let selected_zone: Vec<(GeoPoint, u32)> = selected_id
        .as_ref()
        .and_then(|id| app.story(id))
        .map(|s| s.zone())
        .unwrap_or_default();
    let selected_focus = selected_id
        .as_ref()
        .and_then(|id| app.story(id))
        .and_then(|s| s.focus());

    // Publish click targets before the borrow of `visible` ends.
    let hits: Vec<(u16, u16, String)> = markers
        .iter()
        .filter_map(|m| vp.cell(&m.point, inner).map(|(x, y)| (x, y, m.id.clone())))
        .collect();
    drop(visible);
    app.map_hits = hits;

    let marker_style = app.map_style.marker();
    let title = format!(" WORLD  [{}]  {} ", app.map_style.label(), label);

    let canvas = Canvas::default()
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
                    title,
                    Style::default()
                        .fg(Color::Rgb(180, 210, 235))
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .marker(marker_style)
        .x_bounds(vp.lon)
        .y_bounds(vp.lat)
        .paint(move |ctx: &mut Context| {
            ctx.draw(&Map {
                color: Color::Rgb(58, 68, 80),
                resolution: MapResolution::High,
            });

            // The affected zone of the selected story. A real bbox (GDACS
            // gives us one) is drawn as an actual rectangle — the true shape
            // of the alert area — rather than the circle every other source
            // gets, which is only ever a guess at how wide things are.
            ctx.layer();
            if let Some(place) = &selected_focus {
                match place.bbox {
                    Some([lon_min, lon_max, lat_min, lat_max]) => {
                        ctx.draw(&Rectangle {
                            x: lon_min,
                            y: lat_min,
                            width: (lon_max - lon_min).max(0.2),
                            height: (lat_max - lat_min).max(0.2),
                            color: Color::Rgb(255, 210, 120),
                        });
                    }
                    None => {
                        ctx.draw(&Circle {
                            x: place.point.lon,
                            y: place.point.lat,
                            radius: place.radius_deg.max(0.6),
                            color: Color::Rgb(255, 210, 120),
                        });
                        if place.radius_deg > 1.5 {
                            ctx.draw(&Circle {
                                x: place.point.lon,
                                y: place.point.lat,
                                radius: place.radius_deg * 0.55,
                                color: Color::Rgb(180, 140, 70),
                            });
                        }
                    }
                }
            }
            // Every other place the selected story mentions, linked to the focus.
            if let Some(place) = &selected_focus {
                for (p, _) in &selected_zone {
                    if (p.lat - place.point.lat).abs() < 0.01 && (p.lon - place.point.lon).abs() < 0.01 {
                        continue;
                    }
                    ctx.draw(&CanvasLine {
                        x1: place.point.lon,
                        y1: place.point.lat,
                        x2: p.lon,
                        y2: p.lat,
                        color: Color::Rgb(90, 80, 55),
                    });
                }
            }

            // Story markers. A single braille dot vanishes into the coastline,
            // so each story is stamped as its category character instead.
            ctx.layer();
            for m in &markers {
                if m.selected {
                    continue;
                }
                ctx.print(
                    m.point.lon,
                    m.point.lat,
                    Line::from(Span::styled(
                        m.glyph.to_string(),
                        Style::default()
                            .fg(dim(m.color, m.severity))
                            .add_modifier(if m.severity >= Severity::Severe {
                                Modifier::BOLD
                            } else {
                                Modifier::empty()
                            }),
                    )),
                );
            }

            ctx.layer();
            for m in markers.iter().filter(|m| m.selected) {
                let pts = [(m.point.lon, m.point.lat)];
                ctx.draw(&Points { coords: &pts, color: Color::Rgb(255, 245, 200) });
                ctx.print(
                    m.point.lon,
                    m.point.lat,
                    Line::from(vec![
                        Span::styled(
                            "◆",
                            Style::default()
                                .fg(Color::Rgb(255, 245, 200))
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!(" {}", m.label),
                            Style::default()
                                .fg(Color::Rgb(255, 230, 160))
                                .add_modifier(Modifier::BOLD),
                        ),
                    ]),
                );
            }
        });

    f.render_widget(canvas, area);

    // A zoomed-in view loses all sense of where in the world it's looking —
    // "Ukraine / Russia" or a story's own close-up crop could be anywhere.
    // A small whole-world inset with a rectangle over the current viewport
    // fixes that, the way a photo editor's crop tool shows the full image
    // with the crop box overlaid. Skipped when already showing the world, or
    // when the pane is too small to spare the room without crowding it.
    let zoomed_in = (vp.lon[1] - vp.lon[0]) < 300.0;
    if zoomed_in && inner.width >= 46 && inner.height >= 12 {
        render_inset(f, inner, vp);
    }
}

/// Whole-world locator, overlaid in the map's bottom-right corner.
fn render_inset(f: &mut Frame, host: Rect, vp: Viewport) {
    let inset = Rect {
        x: host.right().saturating_sub(23),
        y: host.bottom().saturating_sub(8),
        width: 22,
        height: 7,
    };
    // A blank backdrop so the host map's own content underneath doesn't show
    // through the gaps between braille dots or block glyphs.
    f.render_widget(ratatui::widgets::Clear, inset);

    let canvas = Canvas::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Plain)
                .border_style(Style::default().fg(Color::Rgb(80, 90, 105)))
                .style(Style::default().bg(Color::Rgb(12, 15, 20))),
        )
        .marker(ratatui::symbols::Marker::Braille)
        .x_bounds([-180.0, 180.0])
        .y_bounds([-90.0, 90.0])
        .paint(move |ctx: &mut Context| {
            ctx.draw(&Map { color: Color::Rgb(70, 80, 92), resolution: MapResolution::Low });
            ctx.layer();
            ctx.draw(&Rectangle {
                x: vp.lon[0],
                y: vp.lat[0],
                width: (vp.lon[1] - vp.lon[0]).max(1.0),
                height: (vp.lat[1] - vp.lat[0]).max(1.0),
                color: Color::Rgb(255, 210, 120),
            });
        });
    f.render_widget(canvas, inset);
}

/// Severity modulates brightness so a glance at the map reads urgency.
fn dim(color: Color, severity: Severity) -> Color {
    let factor = match severity {
        Severity::Critical => 1.0,
        Severity::Severe => 0.88,
        Severity::Elevated => 0.72,
        Severity::Watch => 0.55,
        Severity::Info => 0.42,
    };
    match color {
        Color::Rgb(r, g, b) => Color::Rgb(
            (r as f64 * factor) as u8,
            (g as f64 * factor) as u8,
            (b as f64 * factor) as u8,
        ),
        other => other,
    }
}

/// Convenience used by the map title and marker labels.
pub trait TitleShort {
    fn title_short(&self, max: usize) -> String;
}

impl TitleShort for Story {
    fn title_short(&self, max: usize) -> String {
        let t = self.title.trim();
        if t.chars().count() <= max {
            return t.to_string();
        }
        let cut: String = t.chars().take(max.saturating_sub(1)).collect();
        let at = cut.rfind(' ').unwrap_or(cut.len());
        format!("{}…", &cut[..at])
    }
}
