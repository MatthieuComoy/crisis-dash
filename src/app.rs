//! Application state: what is on screen, what is selected, and how the
//! timeline is filtered. Kept free of rendering and I/O so it stays testable.

use crate::cluster::Clusterer;
use crate::geo::{self, Zone};
use crate::model::{Category, Item, ItemKind, Severity, Story, RETENTION_HOURS};
use crate::sources::{SourceKind, SourceStatus};
use crate::thumb::ThumbState;
use chrono::{DateTime, Duration, Utc};
use std::collections::{BTreeMap, HashMap};

/// A qualifying story the caller (main.rs) should raise as a real-world
/// alert: terminal bell, desktop notification, sound. Carries just enough to
/// write those without looking the story back up.
#[derive(Debug, Clone)]
pub struct PendingAlert {
    pub story_id: String,
    pub title: String,
    pub place: String,
    pub severity: Severity,
    /// Why this one qualified, for the notification text.
    pub reason: AlertReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertReason {
    ElevatedInEurope,
    CriticalAnywhere,
}

impl AlertReason {
    pub fn label(&self) -> &'static str {
        match self {
            AlertReason::ElevatedInEurope => "Europe",
            AlertReason::CriticalAnywhere => "Critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Stories,
    Map,
    Timeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overlay {
    None,
    Sources,
    Help,
    Item,
}

/// Which map glyph style to draw with. Braille is dense and accurate; Dot and
/// Block are plain characters for terminals or fonts that render braille badly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapStyle {
    Braille,
    Dot,
    Block,
    HalfBlock,
}

impl MapStyle {
    pub fn next(self) -> Self {
        match self {
            MapStyle::Braille => MapStyle::Dot,
            MapStyle::Dot => MapStyle::Block,
            MapStyle::Block => MapStyle::HalfBlock,
            MapStyle::HalfBlock => MapStyle::Braille,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MapStyle::Braille => "braille",
            MapStyle::Dot => "dot",
            MapStyle::Block => "block",
            MapStyle::HalfBlock => "half",
        }
    }

    pub fn marker(self) -> ratatui::symbols::Marker {
        use ratatui::symbols::Marker;
        match self {
            MapStyle::Braille => Marker::Braille,
            MapStyle::Dot => Marker::Dot,
            MapStyle::Block => Marker::Block,
            MapStyle::HalfBlock => Marker::HalfBlock,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    /// Recency-weighted corroboration: the default crisis-desk ordering.
    Heat,
    /// Strictly most recently updated.
    Latest,
    /// Worst first.
    Severity,
}

impl SortMode {
    pub fn next(self) -> Self {
        match self {
            SortMode::Heat => SortMode::Latest,
            SortMode::Latest => SortMode::Severity,
            SortMode::Severity => SortMode::Heat,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SortMode::Heat => "heat",
            SortMode::Latest => "latest",
            SortMode::Severity => "severity",
        }
    }
}

pub struct App {
    pub clusterer: Clusterer,
    pub statuses: BTreeMap<String, SourceStatus>,
    /// Selection is tracked by id, not index: the ranking reorders constantly
    /// and the user's choice must survive that.
    pub selected_story: Option<String>,
    pub story_scroll: usize,
    pub timeline_scroll: usize,
    pub timeline_cursor: usize,
    pub focus: Pane,
    pub overlay: Overlay,
    pub filter: Option<Category>,
    pub kind_filter: Option<ItemKind>,
    /// Minimum severity to show. `Some(Watch)` reads as "watch and above".
    pub severity_filter: Option<Severity>,
    pub zone_filter: Option<Zone>,
    pub search: String,
    pub searching: bool,
    pub map_style: MapStyle,
    pub region: usize,
    /// Follow the selected story with the map viewport.
    pub auto_zoom: bool,
    /// Keep the selection pinned to the top-ranked story until the user picks
    /// one themselves. A monitoring dashboard should show the biggest thing
    /// happening, not whatever happened to arrive first.
    pub follow_top: bool,
    pub sort: SortMode,
    pub paused: bool,
    pub log: Vec<(DateTime<Utc>, String)>,
    pub started: DateTime<Utc>,
    pub items_seen: u64,
    pub items_kept: u64,
    pub last_item_at: Option<DateTime<Utc>>,
    /// Click targets published by the last map render, for hit-testing.
    pub map_hits: Vec<(u16, u16, String)>,
    /// Row ranges published by the last story-list render.
    pub list_origin: (u16, u16),
    /// Line offset of each timeline entry's first line, published by the last
    /// timeline render, so a click can be mapped back to an item.
    pub timeline_block_starts: Vec<usize>,
    /// Thumbnails by URL. Populated lazily — only the entry currently on
    /// screen ever gets fetched, never the whole timeline at once.
    pub thumbnails: HashMap<String, ThumbState>,
    /// Master switch for the bell/notification/auto-switch behaviour below.
    pub alerts_enabled: bool,
    /// Alerts raised since the last drain, for `main.rs` to act on (it owns
    /// the OS-level side effects: bell, `notify-send`, a sound player).
    pending_alerts: Vec<PendingAlert>,
    pub should_quit: bool,
}

impl App {
    pub fn new(now: DateTime<Utc>) -> Self {
        App {
            clusterer: Clusterer::with_anchors(now),
            statuses: BTreeMap::new(),
            selected_story: None,
            story_scroll: 0,
            timeline_scroll: 0,
            timeline_cursor: 0,
            focus: Pane::Stories,
            overlay: Overlay::None,
            filter: None,
            kind_filter: None,
            severity_filter: None,
            zone_filter: None,
            search: String::new(),
            searching: false,
            // Braille packs the most resolution into a cell but needs a font
            // that renders the block cleanly, which not every terminal does;
            // half-block is legible everywhere and still gives 2 rows per
            // cell. `m` cycles through all four, braille included.
            map_style: MapStyle::HalfBlock,
            region: 0,
            auto_zoom: true,
            follow_top: true,
            sort: SortMode::Latest,
            paused: false,
            log: Vec::new(),
            started: now,
            items_seen: 0,
            items_kept: 0,
            last_item_at: None,
            map_hits: Vec::new(),
            list_origin: (0, 0),
            timeline_block_starts: Vec::new(),
            thumbnails: HashMap::new(),
            alerts_enabled: true,
            pending_alerts: Vec::new(),
            should_quit: false,
        }
    }

    /// The thumbnail URL of whichever item is on screen right now: the
    /// timeline cursor's entry, since that is the only one ever fetched.
    pub fn wanted_thumbnail(&self) -> Option<String> {
        self.current_item().and_then(|i| i.thumbnail.clone())
    }

    /// Marks a URL as being fetched, so the caller knows whether to spawn a
    /// request. Returns `true` the first time a URL is seen; the actual fetch
    /// happens in `main.rs`, which owns the HTTP client and the async runtime.
    pub fn begin_thumbnail_fetch(&mut self, url: &str) -> bool {
        if self.thumbnails.contains_key(url) {
            return false;
        }
        self.thumbnails.insert(url.to_string(), ThumbState::Loading);
        true
    }

    pub fn log(&mut self, msg: impl Into<String>) {
        self.log.push((Utc::now(), msg.into()));
        if self.log.len() > 500 {
            self.log.drain(..200);
        }
    }

    pub fn ingest(&mut self, items: Vec<Item>) {
        if self.paused {
            return;
        }
        let now = Utc::now();
        let mut switch_to: Option<(Severity, String)> = None;
        for item in items {
            self.items_seen += 1;
            // Judged on the item that is about to create or join a story —
            // `Clusterer::ingest` reports whether that story was empty before
            // this item, i.e. whether this is genuinely a *new* story rather
            // than a new report inside one already being tracked. Only that
            // counts: a fast-moving war stays one bell at its first qualifying
            // report, not one per follow-up headline, no matter how severe.
            let reason = self.alert_reason(&item);
            let (item_title, item_place, item_severity) = (
                item.title.clone(),
                item.place.as_ref().map(|p| p.name.clone()),
                item.severity,
            );
            if let Some((story_id, is_new_story)) = self.clusterer.ingest(item) {
                self.items_kept += 1;
                self.last_item_at = Some(now);
                if is_new_story {
                    if let Some(reason) = reason {
                        if self.alerts_enabled {
                            self.pending_alerts.push(PendingAlert {
                                story_id: story_id.clone(),
                                title: item_title,
                                place: item_place.unwrap_or_else(|| "unlocated".into()),
                                severity: item_severity,
                                reason,
                            });
                            if switch_to.as_ref().is_none_or(|(s, _)| item_severity > *s) {
                                switch_to = Some((item_severity, story_id));
                            }
                        }
                    }
                }
            }
        }
        // Keep the selection valid if its story was merged away.
        if let Some(id) = &self.selected_story {
            if !self.clusterer.stories.contains_key(id) {
                self.selected_story = None;
            }
        }
        // Track the top-ranked story until the user chooses one. Without this
        // the dashboard latches onto whichever source replied first.
        if self.follow_top || self.selected_story.is_none() {
            if let Some(top) = self.visible(now).first().map(|s| s.id.clone()) {
                if self.selected_story.as_deref() != Some(top.as_str()) {
                    self.timeline_scroll = 0;
                    self.timeline_cursor = 0;
                }
                self.selected_story = Some(top.clone());
                if let Some(s) = self.clusterer.stories.get_mut(&top) {
                    s.unread = 0;
                }
            }
        }

        // An alert takes priority over follow-top's pick, and switches even
        // when auto_zoom/focus were left elsewhere by the user.
        if let Some((_, id)) = switch_to {
            self.select_id(&id);
            self.auto_zoom = true;
            // Don't yank focus away from an in-progress search or an open
            // overlay; the selection still switches underneath either way,
            // so it's already there the moment the user is free to look.
            if !self.searching && self.overlay == Overlay::None {
                self.focus = Pane::Timeline;
            }
        }
    }

    /// A report worth a bell/notification/auto-switch: critical anywhere in
    /// the world, or elevated-or-worse with its place in Europe. Requested as
    /// exactly this rule; there is nowhere else it is configured, since
    /// nothing else asked for a different one yet.
    fn alert_reason(&self, item: &Item) -> Option<AlertReason> {
        if item.severity >= Severity::Critical {
            return Some(AlertReason::CriticalAnywhere);
        }
        if item.severity >= Severity::Elevated
            && item.place.as_ref().is_some_and(|p| geo::zone_of(p.point) == Zone::Europe)
        {
            return Some(AlertReason::ElevatedInEurope);
        }
        None
    }

    /// Drains alerts raised since the last call, for `main.rs` to turn into
    /// a bell, a desktop notification and a sound.
    pub fn take_pending_alerts(&mut self) -> Vec<PendingAlert> {
        std::mem::take(&mut self.pending_alerts)
    }

    pub fn update_status(&mut self, status: SourceStatus) {
        self.statuses.insert(status.id.clone(), status);
    }

    /// Stories after filtering, in the current sort order.
    pub fn visible(&self, now: DateTime<Utc>) -> Vec<&Story> {
        let needle = self.search.to_lowercase();
        let mut v: Vec<&Story> = self
            .clusterer
            .stories
            .values()
            .filter(|s| !s.items.is_empty())
            // "No update in two months" applies to every story, anchors
            // included: the clusterer never drops an anchor outright (so a
            // dormant war can wake back up), but a dashboard should not keep
            // showing it once nobody has reported on it in two months.
            .filter(|s| !s.is_stale(now, RETENTION_HOURS))
            .filter(|s| self.filter.map_or(true, |c| s.category == c))
            .filter(|s| self.severity_filter.map_or(true, |min| s.severity >= min))
            .filter(|s| {
                self.zone_filter.map_or(true, |z| {
                    s.focus().is_some_and(|p| geo::zone_of(p.point) == z)
                })
            })
            .filter(|s| {
                if needle.is_empty() {
                    return true;
                }
                s.title.to_lowercase().contains(&needle)
                    || s.items.iter().take(40).any(|i| {
                        i.title.to_lowercase().contains(&needle)
                            || i.place.as_ref().is_some_and(|p| p.name.to_lowercase().contains(&needle))
                    })
            })
            .collect();

        match self.sort {
            SortMode::Heat => v.sort_by(|a, b| {
                b.heat(now)
                    .partial_cmp(&a.heat(now))
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.last_update.cmp(&a.last_update))
            }),
            SortMode::Latest => v.sort_by(|a, b| b.last_update.cmp(&a.last_update)),
            SortMode::Severity => v.sort_by(|a, b| {
                b.severity
                    .cmp(&a.severity)
                    .then(b.last_update.cmp(&a.last_update))
            }),
        }
        v
    }

    pub fn selected_index(&self, visible: &[&Story]) -> Option<usize> {
        let id = self.selected_story.as_ref()?;
        visible.iter().position(|s| &s.id == id)
    }

    pub fn story(&self, id: &str) -> Option<&Story> {
        self.clusterer.stories.get(id)
    }

    /// Items of the selected story after the kind filter, newest first.
    pub fn timeline(&self) -> Vec<&Item> {
        let Some(story) = self.selected_story.as_ref().and_then(|id| self.story(id)) else {
            return Vec::new();
        };
        story
            .items
            .iter()
            .filter(|i| self.kind_filter.map_or(true, |k| i.kind == k))
            .collect()
    }

    /// Select a story by id. Takes an id rather than an index so callers never
    /// have to hold a borrow of the ranked list across the mutation.
    pub fn select_id(&mut self, id: &str) {
        if !self.clusterer.stories.contains_key(id) {
            return;
        }
        self.follow_top = false;
        if self.selected_story.as_deref() != Some(id) {
            self.timeline_scroll = 0;
            self.timeline_cursor = 0;
        }
        self.selected_story = Some(id.to_string());
        self.mark_read();
    }

    fn mark_read(&mut self) {
        if let Some(id) = self.selected_story.clone() {
            if let Some(s) = self.clusterer.stories.get_mut(&id) {
                s.unread = 0;
            }
        }
    }

    pub fn move_selection(&mut self, delta: i32) {
        let now = Utc::now();
        let visible = self.visible(now);
        if visible.is_empty() {
            return;
        }
        let current = self.selected_index(&visible).map(|i| i as i32).unwrap_or(-1);
        let next = (current + delta).clamp(0, visible.len() as i32 - 1) as usize;
        let ids: Vec<String> = visible.iter().map(|s| s.id.clone()).collect();
        drop(visible);
        self.follow_top = false;
        if let Some(id) = ids.get(next) {
            if self.selected_story.as_deref() != Some(id.as_str()) {
                self.timeline_scroll = 0;
                self.timeline_cursor = 0;
            }
            self.selected_story = Some(id.clone());
            self.mark_read();
        }
        // Keep the cursor inside the rendered window.
        if next < self.story_scroll {
            self.story_scroll = next;
        }
    }

    /// Select whichever story has a map marker nearest to a clicked cell.
    pub fn select_nearest_marker(&mut self, col: u16, row: u16) -> bool {
        let mut best: Option<(i32, String)> = None;
        for (x, y, id) in &self.map_hits {
            let dx = *x as i32 - col as i32;
            let dy = (*y as i32 - row as i32) * 2; // Cells are ~twice as tall as wide.
            let d = dx * dx + dy * dy;
            if best.as_ref().map_or(true, |(bd, _)| d < *bd) {
                best = Some((d, id.clone()));
            }
        }
        match best {
            // Within roughly four columns: anything further is a miss, not a pick.
            Some((d, id)) if d <= 64 => {
                self.follow_top = false;
                if self.selected_story.as_deref() != Some(id.as_str()) {
                    self.timeline_scroll = 0;
                    self.timeline_cursor = 0;
                }
                self.selected_story = Some(id);
                self.mark_read();
                true
            }
            _ => false,
        }
    }

    pub fn cycle_filter(&mut self, forward: bool) {
        let all = Category::ALL;
        self.filter = match self.filter {
            None => Some(if forward { all[0] } else { all[all.len() - 1] }),
            Some(c) => {
                let i = all.iter().position(|x| *x == c).unwrap_or(0) as i32;
                let n = all.len() as i32;
                let next = if forward { i + 1 } else { i - 1 };
                if next >= n || next < 0 {
                    None
                } else {
                    Some(all[next as usize])
                }
            }
        };
        self.story_scroll = 0;
    }

    /// Cycles a minimum severity threshold: None ("everything") then Watch,
    /// Elevated, Severe, Critical, each meaning "this and worse". Info is
    /// skipped as a threshold — filtering it out is what "Watch and above"
    /// already does.
    pub fn cycle_severity_filter(&mut self, forward: bool) {
        const LEVELS: [Severity; 4] =
            [Severity::Watch, Severity::Elevated, Severity::Severe, Severity::Critical];
        self.severity_filter = match self.severity_filter {
            None => Some(if forward { LEVELS[0] } else { LEVELS[LEVELS.len() - 1] }),
            Some(cur) => {
                let i = LEVELS.iter().position(|x| *x == cur).unwrap_or(0) as i32;
                let n = LEVELS.len() as i32;
                let next = if forward { i + 1 } else { i - 1 };
                if next >= n || next < 0 {
                    None
                } else {
                    Some(LEVELS[next as usize])
                }
            }
        };
        self.story_scroll = 0;
    }

    /// Cycles the geographic zone filter (Europe, Middle East, Africa, Asia,
    /// Americas, Oceania, Other).
    pub fn cycle_zone_filter(&mut self, forward: bool) {
        let all = Zone::ALL;
        self.zone_filter = match self.zone_filter {
            None => Some(if forward { all[0] } else { all[all.len() - 1] }),
            Some(z) => {
                let i = all.iter().position(|x| *x == z).unwrap_or(0) as i32;
                let n = all.len() as i32;
                let next = if forward { i + 1 } else { i - 1 };
                if next >= n || next < 0 {
                    None
                } else {
                    Some(all[next as usize])
                }
            }
        };
        self.story_scroll = 0;
    }

    pub fn cycle_kind_filter(&mut self) {
        self.kind_filter = match self.kind_filter {
            None => Some(ItemKind::Video),
            Some(ItemKind::Video) => Some(ItemKind::Alert),
            Some(ItemKind::Alert) => Some(ItemKind::Article),
            Some(ItemKind::Article) => Some(ItemKind::Report),
            Some(ItemKind::Report) => None,
        };
        self.timeline_scroll = 0;
        self.timeline_cursor = 0;
    }

    pub fn scroll_timeline(&mut self, delta: i32) {
        let len = self.timeline().len();
        if len == 0 {
            return;
        }
        let next = (self.timeline_cursor as i32 + delta).clamp(0, len as i32 - 1) as usize;
        self.timeline_cursor = next;
        if next < self.timeline_scroll {
            self.timeline_scroll = next;
        }
    }

    /// The item under the timeline cursor, used by "open in browser".
    pub fn current_item(&self) -> Option<&Item> {
        let tl = self.timeline();
        tl.get(self.timeline_cursor).copied()
    }

    pub fn healthy_sources(&self) -> (usize, usize) {
        let total = self.statuses.len();
        let ok = self.statuses.values().filter(|s| s.healthy()).count();
        (ok, total)
    }

    pub fn source_kind_counts(&self) -> BTreeMap<SourceKind, (usize, usize)> {
        let mut m: BTreeMap<SourceKind, (usize, usize)> = BTreeMap::new();
        for s in self.statuses.values() {
            let e = m.entry(s.kind).or_insert((0, 0));
            e.1 += 1;
            if s.healthy() {
                e.0 += 1;
            }
        }
        m
    }

    /// Category totals across visible stories, for the header strip.
    pub fn category_counts(&self, now: DateTime<Utc>) -> Vec<(Category, usize)> {
        let mut counts: BTreeMap<Category, usize> = BTreeMap::new();
        for s in self.visible(now) {
            *counts.entry(s.category).or_insert(0) += 1;
        }
        let mut v: Vec<(Category, usize)> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        v
    }

    pub fn worst_severity(&self, now: DateTime<Utc>) -> Severity {
        self.clusterer
            .stories
            .values()
            .filter(|s| !s.items.is_empty() && !s.is_stale(now, 12))
            .map(|s| s.severity)
            .max()
            .unwrap_or(Severity::Info)
    }

    /// Items across all stories from the last few minutes — the live ticker.
    pub fn newest_items(&self, now: DateTime<Utc>, limit: usize) -> Vec<&Item> {
        let mut v: Vec<&Item> = self
            .clusterer
            .stories
            .values()
            .flat_map(|s| s.items.iter().take(3))
            .filter(|i| now - i.published < Duration::hours(6))
            .collect();
        v.sort_by(|a, b| b.published.cmp(&a.published));
        v.truncate(limit);
        v
    }
}

/// Compact relative time: the timeline is dense and "3h ago" wastes columns.
pub fn ago(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let d = now - then;
    let secs = d.num_seconds();
    if secs < 0 {
        return "now".into();
    }
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify;
    use crate::model::ItemKind;

    fn mk_item(title: &str, cat: Category, at: DateTime<Utc>) -> Item {
        Item {
            id: crate::sources::parsers::item_id(title),
            kind: ItemKind::Article,
            source: "test".into(),
            title: title.into(),
            snippet: String::new(),
            url: "https://example.org/x".into(),
            published: at,
            fetched: at,
            category: cat,
            severity: Severity::Elevated,
            place: None,
            tokens: classify::tokenize(title),
            facts: Vec::new(),
            relevance: 9,
            event_key: None,
            thumbnail: None,
        }
    }

    /// Like `mk_item`, but with a chosen severity and place — the two things
    /// the alert rule actually looks at.
    fn mk_alertable(
        title: &str,
        severity: Severity,
        place: Option<(&str, f64, f64)>,
        at: DateTime<Utc>,
    ) -> Item {
        let mut item = mk_item(title, Category::ArmedConflict, at);
        item.severity = severity;
        item.place = place.map(|(name, lat, lon)| crate::model::Place {
            name: name.to_string(),
            point: crate::model::GeoPoint::new(lat, lon),
            radius_deg: 1.0,
            bbox: None,
        });
        item
    }

    #[test]
    fn alert_fires_for_new_elevated_story_in_europe() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.ingest(vec![mk_alertable(
            "Clashes reported near the border",
            Severity::Elevated,
            Some(("Poland", 51.9, 19.1)),
            now,
        )]);
        let alerts = app.take_pending_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].reason, AlertReason::ElevatedInEurope);
        // The "switch directly to it" half of the request.
        assert_eq!(app.selected_story.as_deref(), Some(alerts[0].story_id.as_str()));
        assert!(!app.follow_top, "a deliberate switch should stop auto-following");
    }

    #[test]
    fn alert_fires_for_critical_anywhere_outside_europe() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.ingest(vec![mk_alertable(
            "Massacre reported after overnight assault",
            Severity::Critical,
            Some(("Sudan", 15.5, 32.5)),
            now,
        )]);
        let alerts = app.take_pending_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].reason, AlertReason::CriticalAnywhere);
    }

    #[test]
    fn no_alert_below_elevated_or_outside_europe_below_critical() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.ingest(vec![
            mk_alertable("Minor incident in France", Severity::Watch, Some(("France", 46.6, 2.3)), now),
            mk_alertable("Fighting continues in Sudan", Severity::Severe, Some(("Sudan", 15.5, 32.5)), now),
        ]);
        assert!(app.take_pending_alerts().is_empty());
    }

    #[test]
    fn a_follow_up_on_the_same_story_never_alerts_again() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.ingest(vec![mk_alertable(
            "Border incident escalates",
            Severity::Elevated,
            Some(("Poland", 51.9, 19.1)),
            now,
        )]);
        assert_eq!(app.take_pending_alerts().len(), 1, "the story's first report alerts");

        // A later, even more severe report on that *same* story must not
        // alert again — only the story's own creation counts, no matter how
        // the situation develops afterwards or how much time has passed.
        app.ingest(vec![mk_alertable(
            "Border incident escalates further, casualties reported",
            Severity::Critical,
            Some(("Poland", 51.9, 19.1)),
            now + Duration::hours(5),
        )]);
        assert!(
            app.take_pending_alerts().is_empty(),
            "a follow-up in an already-tracked story must never alert, however severe"
        );
    }

    #[test]
    fn alerts_can_be_turned_off() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.alerts_enabled = false;
        app.ingest(vec![mk_alertable(
            "Massacre reported after overnight assault",
            Severity::Critical,
            Some(("Sudan", 15.5, 32.5)),
            now,
        )]);
        assert!(app.take_pending_alerts().is_empty());
    }

    #[test]
    fn a_qualifying_item_joining_an_existing_story_never_alerts() {
        // Regression: alerting used to judge each new item on its own merits
        // regardless of whether it created a story or joined one, so a
        // fast-moving war buzzed on every single severe headline (throttled
        // only by a time-based cooldown). The rule now is stricter and
        // simpler: only a story's *first* qualifying report ever alerts.
        let now = Utc::now();
        let mut app = App::new(now);

        app.ingest(vec![mk_alertable(
            "Massacre reported after overnight assault",
            Severity::Critical,
            Some(("Sudan", 15.5, 32.5)),
            now,
        )]);
        assert_eq!(app.take_pending_alerts().len(), 1, "the story's first report alerts");

        // A second, independently-qualifying report that joins the same
        // story (same place, same vocabulary) must not alert again.
        app.ingest(vec![mk_alertable(
            "Second massacre reported in neighbouring town",
            Severity::Critical,
            Some(("Sudan", 15.5, 32.5)),
            now,
        )]);
        assert!(
            app.take_pending_alerts().is_empty(),
            "a new report joining an already-tracked story must not alert, however severe"
        );
    }

    #[test]
    fn an_anchors_first_ever_report_counts_as_a_new_story() {
        // Anchors (Ukraine, Gaza, …) exist as empty placeholders from
        // startup so their history survives quiet periods. From outside this
        // module nothing about an anchor is visible until it has a report,
        // so its first one must alert exactly like any other new story.
        let now = Utc::now();
        let mut app = App::new(now);
        let mut first = mk_alertable(
            "Russian missile strike hits Kyiv overnight",
            Severity::Critical,
            Some(("Ukraine", 49.0, 32.0)),
            now,
        );
        // The real pipeline sets this fact via `classify::match_anchor`;
        // set it directly here to exercise anchor routing specifically,
        // rather than relying on plain similarity to join the two items.
        first.facts.push(("anchor".into(), "ukraine-war".into()));
        app.ingest(vec![first]);
        assert_eq!(app.take_pending_alerts().len(), 1, "an anchor's first report is a new story");

        // Its second report is not — the anchor already existed by then.
        let mut second = mk_alertable(
            "Second missile strike reported near Kharkiv",
            Severity::Critical,
            Some(("Ukraine", 49.0, 32.0)),
            now,
        );
        second.facts.push(("anchor".into(), "ukraine-war".into()));
        app.ingest(vec![second]);
        assert!(app.take_pending_alerts().is_empty(), "the anchor already existed");
    }

    #[test]
    fn selection_survives_reordering() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.ingest(vec![
            mk_item("Cyclone makes landfall in Mozambique overnight", Category::NaturalDisaster, now - Duration::hours(2)),
            mk_item("Wildfires force evacuations across Greece", Category::NaturalDisaster, now - Duration::hours(1)),
        ]);
        let chosen = app.visible(now)[1].id.clone();
        app.select_id(&chosen);

        // A burst of new items reshuffles the ranking.
        app.ingest(vec![mk_item(
            "Cyclone death toll rises sharply in Mozambique",
            Category::NaturalDisaster,
            now,
        )]);
        assert_eq!(app.selected_story.as_deref(), Some(chosen.as_str()));
    }

    #[test]
    fn category_filter_narrows_the_list() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.ingest(vec![
            mk_item("Earthquake strikes off the coast of Japan", Category::Earthquake, now),
            mk_item("Central bank holds interest rates steady", Category::Economy, now),
        ]);
        assert_eq!(app.visible(now).len(), 2);
        app.filter = Some(Category::Earthquake);
        assert_eq!(app.visible(now).len(), 1);
    }

    #[test]
    fn search_matches_titles() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.ingest(vec![
            mk_item("Flooding submerges villages in Bangladesh", Category::NaturalDisaster, now),
            mk_item("Protests erupt in the capital over fuel prices", Category::Unrest, now),
        ]);
        app.search = "flood".into();
        assert_eq!(app.visible(now).len(), 1);
    }

    #[test]
    fn ago_is_compact() {
        let now = Utc::now();
        assert_eq!(ago(now, now - Duration::seconds(5)), "5s");
        assert_eq!(ago(now, now - Duration::minutes(7)), "7m");
        assert_eq!(ago(now, now - Duration::hours(3)), "3h");
        assert_eq!(ago(now, now - Duration::days(2)), "2d");
    }

    #[test]
    fn moving_selection_clamps_at_the_ends() {
        let now = Utc::now();
        let mut app = App::new(now);
        app.ingest(vec![
            mk_item("Volcano erupts in Iceland near Grindavik", Category::NaturalDisaster, now),
            mk_item("Landslide buries homes in Peru", Category::NaturalDisaster, now),
        ]);
        app.move_selection(-5);
        assert!(app.selected_story.is_some());
        app.move_selection(99);
        let visible = app.visible(now);
        assert_eq!(app.selected_index(&visible), Some(visible.len() - 1));
    }
}
