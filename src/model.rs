//! Core domain types: categories, severities, geo points, ingested items and clustered stories.

use chrono::{DateTime, Duration, Utc};
use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// How long a story may go without an update before it is purged. Shared by
/// the clusterer (drops it), the disk snapshot (won't write it back) and the
/// story list (hides it even if it is an anchor, which never gets dropped).
/// Two months, expressed as hours since that's what `chrono::Duration` wants.
pub const RETENTION_HOURS: i64 = 24 * 60;

/// Broad bucket a story belongs to. Deliberately coarse: the *story* carries the
/// specific identity ("Ukraine war"), the category carries the colour and filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub enum Category {
    UkraineWar,
    MiddleEast,
    ArmedConflict,
    Terrorism,
    Unrest,
    Earthquake,
    NaturalDisaster,
    WeatherClimate,
    Health,
    Migration,
    Cyber,
    Economy,
    Politics,
    Other,
}

impl Category {
    pub const ALL: [Category; 14] = [
        Category::UkraineWar,
        Category::MiddleEast,
        Category::ArmedConflict,
        Category::Terrorism,
        Category::Unrest,
        Category::Earthquake,
        Category::NaturalDisaster,
        Category::WeatherClimate,
        Category::Health,
        Category::Migration,
        Category::Cyber,
        Category::Economy,
        Category::Politics,
        Category::Other,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Category::UkraineWar => "Ukraine war",
            Category::MiddleEast => "Middle East",
            Category::ArmedConflict => "Armed conflict",
            Category::Terrorism => "Terrorism",
            Category::Unrest => "Unrest / coup",
            Category::Earthquake => "Earthquake",
            Category::NaturalDisaster => "Natural disaster",
            Category::WeatherClimate => "Weather / climate",
            Category::Health => "Health / epidemic",
            Category::Migration => "Migration",
            Category::Cyber => "Cyber",
            Category::Economy => "Economy",
            Category::Politics => "Politics / diplomacy",
            Category::Other => "Other",
        }
    }

    /// Short tag used in dense list rows.
    pub fn tag(&self) -> &'static str {
        match self {
            Category::UkraineWar => "UKR",
            Category::MiddleEast => "MEA",
            Category::ArmedConflict => "WAR",
            Category::Terrorism => "TER",
            Category::Unrest => "UNR",
            Category::Earthquake => "EQK",
            Category::NaturalDisaster => "NAT",
            Category::WeatherClimate => "WXC",
            Category::Health => "HLT",
            Category::Migration => "MIG",
            Category::Cyber => "CYB",
            Category::Economy => "ECO",
            Category::Politics => "POL",
            Category::Other => "GEN",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Category::UkraineWar => Color::Rgb(255, 213, 0),
            Category::MiddleEast => Color::Rgb(255, 122, 89),
            Category::ArmedConflict => Color::Rgb(255, 85, 85),
            Category::Terrorism => Color::Rgb(214, 63, 120),
            Category::Unrest => Color::Rgb(226, 135, 67),
            Category::Earthquake => Color::Rgb(175, 122, 255),
            Category::NaturalDisaster => Color::Rgb(90, 196, 255),
            Category::WeatherClimate => Color::Rgb(86, 214, 190),
            Category::Health => Color::Rgb(126, 219, 120),
            Category::Migration => Color::Rgb(197, 168, 128),
            Category::Cyber => Color::Rgb(120, 170, 255),
            Category::Economy => Color::Rgb(160, 200, 110),
            Category::Politics => Color::Rgb(160, 160, 190),
            Category::Other => Color::Rgb(130, 130, 130),
        }
    }

    /// Marker drawn on the world map.
    pub fn glyph(&self) -> &'static str {
        match self {
            Category::UkraineWar | Category::ArmedConflict => "#",
            Category::MiddleEast => "*",
            Category::Terrorism => "!",
            Category::Unrest => "%",
            Category::Earthquake => "@",
            Category::NaturalDisaster => "~",
            Category::WeatherClimate => "&",
            Category::Health => "+",
            Category::Migration => ">",
            Category::Cyber => "$",
            Category::Economy => "=",
            Category::Politics => "o",
            Category::Other => ".",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
pub enum Severity {
    Info = 0,
    Watch = 1,
    Elevated = 2,
    Severe = 3,
    Critical = 4,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Info => "INFO",
            Severity::Watch => "WATCH",
            Severity::Elevated => "ELEV",
            Severity::Severe => "SEVERE",
            Severity::Critical => "CRIT",
        }
    }

    pub fn color(&self) -> Color {
        match self {
            Severity::Info => Color::Rgb(120, 130, 140),
            Severity::Watch => Color::Rgb(120, 190, 220),
            Severity::Elevated => Color::Rgb(240, 200, 90),
            Severity::Severe => Color::Rgb(255, 140, 60),
            Severity::Critical => Color::Rgb(255, 70, 70),
        }
    }

    pub fn weight(&self) -> f64 {
        match self {
            Severity::Info => 1.0,
            Severity::Watch => 1.4,
            Severity::Elevated => 2.0,
            Severity::Severe => 3.0,
            Severity::Critical => 4.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum ItemKind {
    /// A news article scraped from an RSS/Atom feed.
    Article,
    /// A machine-generated alert with authoritative coordinates (GDACS, USGS, EONET).
    Alert,
    /// A video published on a news organisation's YouTube channel.
    Video,
    /// A humanitarian situation report.
    Report,
}

impl ItemKind {
    pub fn glyph(&self) -> &'static str {
        match self {
            ItemKind::Article => "▪",
            ItemKind::Alert => "⚠",
            ItemKind::Video => "▶",
            ItemKind::Report => "◆",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    pub lat: f64,
    pub lon: f64,
}

impl GeoPoint {
    pub fn new(lat: f64, lon: f64) -> Self {
        Self { lat, lon }
    }

    /// Great-circle distance in kilometres.
    pub fn distance_km(&self, other: &GeoPoint) -> f64 {
        let r = 6371.0_f64;
        let (p1, p2) = (self.lat.to_radians(), other.lat.to_radians());
        let dp = (other.lat - self.lat).to_radians();
        let dl = (other.lon - self.lon).to_radians();
        let a = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
        2.0 * r * a.sqrt().asin()
    }
}

/// A named location attached to an item, resolved either from the source's own
/// coordinates or from the gazetteer lookup over the headline text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub name: String,
    pub point: GeoPoint,
    /// Roughly how wide the affected zone is, in degrees. Countries are large,
    /// earthquake epicentres are small.
    pub radius_deg: f64,
    /// The real affected-area box, when the source hands us one (GDACS does).
    /// `[lon_min, lon_max, lat_min, lat_max]`. Drawn as an actual rectangle on
    /// the map instead of the `radius_deg` circle, which is a guess for
    /// everything that doesn't carry real bounds.
    #[serde(default)]
    pub bbox: Option<[f64; 4]>,
}

/// One atomic piece of information from one source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub source: String,
    pub title: String,
    pub snippet: String,
    pub url: String,
    pub published: DateTime<Utc>,
    pub fetched: DateTime<Utc>,
    pub category: Category,
    pub severity: Severity,
    pub place: Option<Place>,
    /// Normalised significant tokens, used for clustering.
    pub tokens: Vec<String>,
    /// Source-specific facts worth showing: magnitude, alert level, channel name.
    pub facts: Vec<(String, String)>,
    /// How strongly the classifier matched. Low scores mean "probably not a crisis".
    #[serde(default)]
    pub relevance: u32,
    /// Registry id of the physical event this item reports (GDACS event id,
    /// USGS event id, EONET id). Items sharing one are the same event by
    /// definition, so they bypass similarity clustering entirely.
    #[serde(default)]
    pub event_key: Option<String>,
    /// URL of an image worth showing alongside this entry — a video's cover
    /// frame, a photo attached to a social post, a hazard map. Fetched lazily
    /// and only for whichever entry is on screen; see `thumb.rs`.
    #[serde(default)]
    pub thumbnail: Option<String>,
}

/// A cluster of items that describe the same unfolding situation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Story {
    pub id: String,
    pub title: String,
    pub category: Category,
    /// Set when this story is one of the predefined long-running situations.
    pub anchor: Option<String>,
    /// Newest first.
    pub items: Vec<Item>,
    #[serde(default)]
    pub seen_ids: HashSet<String>,
    /// Token -> occurrences, the story's centroid for similarity matching.
    #[serde(default)]
    pub tokens: HashMap<String, u32>,
    /// Place name -> (point, occurrences).
    #[serde(default)]
    pub places: HashMap<String, (GeoPoint, u32)>,
    pub first_seen: DateTime<Utc>,
    pub last_update: DateTime<Utc>,
    pub severity: Severity,
    /// Items added since the user last looked at this story.
    #[serde(default)]
    pub unread: usize,
    /// Fixed location for anchored stories. A long-running war is where it is,
    /// regardless of which countries this hour's commentary mentions.
    #[serde(default)]
    pub home: Option<Place>,
}

impl Story {
    pub fn new(id: String, item: &Item) -> Self {
        let mut s = Story {
            id,
            title: item.title.clone(),
            category: item.category,
            anchor: None,
            items: Vec::new(),
            seen_ids: HashSet::new(),
            tokens: HashMap::new(),
            places: HashMap::new(),
            first_seen: item.published,
            last_update: item.published,
            severity: item.severity,
            unread: 0,
            home: None,
        };
        s.absorb(item.clone());
        s
    }

    pub fn anchored(
        id: &str,
        title: &str,
        category: Category,
        home: Option<Place>,
        now: DateTime<Utc>,
    ) -> Self {
        Story {
            id: id.to_string(),
            title: title.to_string(),
            category,
            anchor: Some(id.to_string()),
            items: Vec::new(),
            seen_ids: HashSet::new(),
            tokens: HashMap::new(),
            places: HashMap::new(),
            first_seen: now,
            last_update: now,
            severity: Severity::Info,
            unread: 0,
            home,
        }
    }

    pub fn has(&self, id: &str) -> bool {
        self.seen_ids.contains(id)
    }

    /// Insert an item, keeping the timeline sorted newest-first.
    pub fn absorb(&mut self, item: Item) {
        if !self.seen_ids.insert(item.id.clone()) {
            return;
        }
        for t in &item.tokens {
            *self.tokens.entry(t.clone()).or_insert(0) += 1;
        }
        if let Some(p) = &item.place {
            let e = self
                .places
                .entry(p.name.clone())
                .or_insert((p.point, 0));
            e.1 += 1;
        }
        if item.severity > self.severity {
            self.severity = item.severity;
        }
        if item.published > self.last_update {
            self.last_update = item.published;
            // An alert with an authoritative title re-titles the story; articles do not,
            // so a long-running story keeps a stable name.
            if self.anchor.is_none() && item.kind == ItemKind::Alert {
                self.title = item.title.clone();
            }
        }
        if item.published < self.first_seen {
            self.first_seen = item.published;
        }
        let pos = self
            .items
            .binary_search_by(|probe| item.published.cmp(&probe.published))
            .unwrap_or_else(|e| e);
        self.items.insert(pos, item);
        self.unread += 1;
    }

    /// Drop the oldest items so a long-running story stays bounded.
    pub fn trim(&mut self, max: usize) {
        if self.items.len() > max {
            for gone in self.items.drain(max..) {
                self.seen_ids.remove(&gone.id);
            }
        }
    }

    /// The story's best guess at a single point on the map: the most frequently
    /// mentioned place, tie-broken by the most recent item that has coordinates.
    pub fn focus(&self) -> Option<Place> {
        // An anchored story's location is fixed; otherwise it drifts to
        // whichever country this hour's commentary happened to name.
        if let Some(home) = &self.home {
            return Some(home.clone());
        }
        let best = self
            .places
            .iter()
            .max_by_key(|(_, (_, n))| *n)
            .map(|(name, (pt, _))| (name.clone(), *pt));
        let (name, point) = best?;
        // The most recent item naming this place carries the most current
        // read on the affected area — including its real bbox, if it has one.
        let matching = self
            .items
            .iter()
            .filter_map(|i| i.place.as_ref())
            .find(|p| p.name == name);
        let radius = matching.map(|p| p.radius_deg).unwrap_or(3.0);
        let bbox = matching.and_then(|p| p.bbox);
        Some(Place { name, point, radius_deg: radius, bbox })
    }

    /// Every distinct place mentioned, for drawing the affected zone.
    pub fn zone(&self) -> Vec<(GeoPoint, u32)> {
        self.places.values().copied().collect()
    }

    pub fn sources(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .items
            .iter()
            .map(|i| i.source.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        v.sort();
        v
    }

    pub fn video_count(&self) -> usize {
        self.items.iter().filter(|i| i.kind == ItemKind::Video).count()
    }

    /// Ranking score. Recency dominates, then corroboration across distinct
    /// sources, then severity. A story nobody has mentioned for a day sinks.
    pub fn heat(&self, now: DateTime<Utc>) -> f64 {
        let mut recency = 0.0;
        for item in self.items.iter().take(60) {
            let hours = (now - item.published).num_minutes() as f64 / 60.0;
            if hours < 0.0 {
                recency += 1.0;
            } else {
                recency += 1.0 / (1.0 + hours / 6.0);
            }
        }
        let diversity = (self.sources().len() as f64).sqrt();
        let staleness = {
            let h = (now - self.last_update).num_minutes() as f64 / 60.0;
            1.0 / (1.0 + (h / 12.0).max(0.0))
        };
        recency * diversity * self.severity.weight() * staleness
    }

    pub fn is_stale(&self, now: DateTime<Utc>, hours: i64) -> bool {
        now - self.last_update > Duration::hours(hours)
    }
}
