//! Groups incoming items into stories.
//!
//! Two mechanisms work together. Anchors capture the long-running situations a
//! crisis desk tracks by name, so "Russia-Ukraine war" stays one timeline
//! instead of splintering into a story per headline. Everything else is
//! clustered by similarity, so an unexpected event — a coup, a derailment —
//! still assembles its own timeline as corroborating reports arrive.

use crate::classify;
use crate::model::{Category, Item, Story};
use chrono::{DateTime, Utc};
use std::collections::HashMap;

pub struct Clusterer {
    /// Stories keyed by id, including the anchors.
    pub stories: HashMap<String, Story>,
    /// Similarity a free-form item needs to join an existing story.
    pub join_threshold: f64,
    /// Items kept per story.
    pub max_items_per_story: usize,
    /// Stories with no update for this long are dropped entirely.
    pub retire_after_hours: i64,
    next_id: u64,
}

impl Default for Clusterer {
    fn default() -> Self {
        Self {
            stories: HashMap::new(),
            join_threshold: 0.34,
            max_items_per_story: 400,
            retire_after_hours: crate::model::RETENTION_HOURS,
            next_id: 1,
        }
    }
}

impl Clusterer {
    /// Create the anchor stories so they exist (and are visible) before any
    /// matching item arrives.
    pub fn with_anchors(now: DateTime<Utc>) -> Self {
        let mut c = Self::default();
        for a in classify::ANCHOR_LIST.iter() {
            let home = crate::geo::lookup(&a.home);
            c.stories
                .entry(a.id.clone())
                .or_insert_with(|| Story::anchored(&a.id, &a.title, a.category, home, now));
        }
        c
    }

    /// Route one item. Returns the id of the story it landed in and whether
    /// that story was empty before this item — i.e. whether this is the
    /// story's first-ever report, not merely a new id — or `None` if the item
    /// was a duplicate. An anchor (Ukraine, Gaza, …) exists as an empty
    /// placeholder from startup, so its first real item reports `is_new` too:
    /// nothing about it was visible or actionable before that item arrived,
    /// which is what "a new story appeared" means from outside this module.
    pub fn ingest(&mut self, item: Item) -> Option<(String, bool)> {
        let id = item.id.clone();
        if self.stories.values().any(|s| s.has(&id)) {
            return None;
        }

        // Some registries list long-running situations with their *start* date:
        // EONET wildfires and ReliefWeb disasters can be months old. Ingesting
        // them only for `maintain` to retire them minutes later makes the story
        // count flap on every cycle, so reject them at the door. Items joining an
        // anchor are kept regardless — an anchor is the historical record.
        let is_anchored = item.facts.iter().any(|(k, _)| k == "anchor");
        if !is_anchored {
            let age = Utc::now() - item.published;
            if age.num_hours() > self.retire_after_hours {
                return None;
            }
        }

        // 1. A registry event id is ground truth: two USGS reports of the same
        //    quake are the same event, and two different quakes never are, no
        //    matter how alike their formulaic titles look.
        if let Some(key) = item.event_key.clone() {
            let story_id = format!("ev-{key}");
            let is_new = match self.stories.get_mut(&story_id) {
                Some(story) => {
                    let was_empty = story.items.is_empty();
                    story.absorb(item);
                    was_empty
                }
                None => {
                    let mut story = Story::new(story_id.clone(), &item);
                    story.id = story_id.clone();
                    self.stories.insert(story_id.clone(), story);
                    true
                }
            };
            return Some((story_id, is_new));
        }

        // 2. Explicit anchor routing: the classifier decided this belongs to a
        //    named situation.
        if let Some(anchor_id) = item.facts.iter().find(|(k, _)| k == "anchor").map(|(_, v) | v.clone()) {
            if let Some(story) = self.stories.get_mut(&anchor_id) {
                let was_empty = story.items.is_empty();
                story.absorb(item);
                return Some((anchor_id, was_empty));
            }
        }

        // 3. Otherwise find the most similar open story. `best_match` only
        //    ever matches a story that already has items (see its guard
        //    below), so joining one here is never this story's first report.
        let best = self.best_match(&item);
        match best {
            Some((story_id, _score)) => {
                if let Some(story) = self.stories.get_mut(&story_id) {
                    story.absorb(item);
                    return Some((story_id, false));
                }
                None
            }
            None => {
                let sid = self.mint_id(&item);
                let story = Story::new(sid.clone(), &item);
                self.stories.insert(sid.clone(), story);
                Some((sid, true))
            }
        }
    }

    fn mint_id(&mut self, item: &Item) -> String {
        let n = self.next_id;
        self.next_id += 1;
        format!("s{n:05}-{}", &item.id[..6])
    }

    /// Highest-scoring story above the join threshold.
    fn best_match(&self, item: &Item) -> Option<(String, f64)> {
        let mut best: Option<(String, f64)> = None;
        for (id, story) in &self.stories {
            // An empty anchor should not swallow unrelated items just because
            // its token set is vacuous.
            if story.items.is_empty() {
                continue;
            }
            // A registry event owns its timeline; free-text items never join it.
            if id.starts_with("ev-") {
                continue;
            }
            let score = similarity(item, story);
            if score >= self.join_threshold && best.as_ref().map_or(true, |(_, b)| score > *b) {
                best = Some((id.clone(), score));
            }
        }
        best
    }

    /// Housekeeping: trim long timelines, drop stories nobody is updating, and
    /// remove empty non-anchor stories.
    pub fn maintain(&mut self, now: DateTime<Utc>) {
        let retire = self.retire_after_hours;
        let max_items = self.max_items_per_story;
        self.stories.retain(|_, s| {
            if s.anchor.is_some() {
                return true; // Anchors persist even while quiet.
            }
            if s.items.is_empty() {
                return false;
            }
            !s.is_stale(now, retire)
        });
        for story in self.stories.values_mut() {
            story.trim(max_items);
        }
        self.merge_duplicates();
    }

    /// Two independently created stories can converge as they accumulate items
    /// (the same event reported with different wording). Fold the younger into
    /// the older when they become near-identical.
    fn merge_duplicates(&mut self) {
        let ids: Vec<String> = self
            .stories
            .iter()
            .filter(|(id, s)| s.anchor.is_none() && !s.items.is_empty() && !id.starts_with("ev-"))
            .map(|(id, _)| id.clone())
            .collect();

        let mut merges: Vec<(String, String)> = Vec::new();
        for (i, a_id) in ids.iter().enumerate() {
            for b_id in ids.iter().skip(i + 1) {
                let (Some(a), Some(b)) = (self.stories.get(a_id), self.stories.get(b_id)) else {
                    continue;
                };
                if a.category != b.category {
                    continue;
                }
                if story_similarity(a, b) > 0.55 && near_in_space(a, b) {
                    // Keep the story that started first.
                    if a.first_seen <= b.first_seen {
                        merges.push((a_id.clone(), b_id.clone()));
                    } else {
                        merges.push((b_id.clone(), a_id.clone()));
                    }
                }
            }
        }

        for (keep, drop) in merges {
            if keep == drop || !self.stories.contains_key(&drop) || !self.stories.contains_key(&keep) {
                continue;
            }
            let Some(victim) = self.stories.remove(&drop) else { continue };
            if let Some(target) = self.stories.get_mut(&keep) {
                for item in victim.items {
                    target.absorb(item);
                }
            }
        }
    }

    pub fn story_count(&self) -> usize {
        self.stories.len()
    }

    pub fn item_count(&self) -> usize {
        self.stories.values().map(|s| s.items.len()).sum()
    }

}

/// How well an item fits a story: shared vocabulary, same category, same place,
/// and whether the story is still active.
fn similarity(item: &Item, story: &Story) -> f64 {
    if item.tokens.is_empty() {
        return 0.0;
    }

    // Vocabulary overlap, weighted so a token the story mentions often counts
    // for more than a one-off.
    let story_total: u32 = story.tokens.values().sum();
    if story_total == 0 {
        return 0.0;
    }
    let mut shared = 0.0;
    for t in &item.tokens {
        if let Some(count) = story.tokens.get(t) {
            shared += 1.0 + (*count as f64).ln_1p();
        }
    }
    let denom = (item.tokens.len() as f64) + (story.tokens.len() as f64).sqrt();
    let lexical = (shared / denom).min(1.0);

    let category_bonus = if item.category == story.category {
        0.12
    } else if item.category == Category::Other {
        0.0
    } else {
        // A different, confident category is evidence against joining.
        -0.15
    };

    let geo_bonus = match (&item.place, story.focus()) {
        (Some(a), Some(b)) => {
            let d = a.point.distance_km(&b.point);
            // A physical event happens in one place. Reports of two different
            // earthquakes are worded almost identically, so without this the
            // lexical score alone merges every quake on Earth into one story.
            if is_physical(item.category) && is_physical(story.category) && d > 600.0 {
                return 0.0;
            }
            if d < 150.0 {
                0.16
            } else if d < 600.0 {
                0.08
            } else if d > 3000.0 {
                -0.12
            } else {
                0.0
            }
        }
        _ => 0.0,
    };

    // Events are bounded in time; a report three days later is usually a
    // different story even when it uses the same words.
    let time_penalty = {
        let gap = (item.published - story.last_update).num_hours().abs();
        if gap > 72 {
            -0.20
        } else if gap > 36 {
            -0.08
        } else {
            0.0
        }
    };

    (lexical + category_bonus + geo_bonus + time_penalty).clamp(0.0, 1.0)
}

/// Categories whose events occupy a single location at a single time.
fn is_physical(c: Category) -> bool {
    matches!(c, Category::Earthquake | Category::NaturalDisaster)
}

/// Cosine-ish overlap between two stories' vocabularies.
fn story_similarity(a: &Story, b: &Story) -> f64 {
    if a.tokens.is_empty() || b.tokens.is_empty() {
        return 0.0;
    }
    let shared: u32 = a
        .tokens
        .iter()
        .filter_map(|(t, ca)| b.tokens.get(t).map(|cb| (*ca).min(*cb)))
        .sum();
    let total_a: u32 = a.tokens.values().sum();
    let total_b: u32 = b.tokens.values().sum();
    (2.0 * shared as f64) / (total_a + total_b).max(1) as f64
}

fn near_in_space(a: &Story, b: &Story) -> bool {
    match (a.focus(), b.focus()) {
        (Some(pa), Some(pb)) => {
            let limit = if is_physical(a.category) { 400.0 } else { 800.0 };
            pa.point.distance_km(&pb.point) < limit
        }
        // Without coordinates, fall back to trusting the vocabulary check.
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemKind, Severity};
    use chrono::Duration;

    fn item(title: &str, cat: Category, at: DateTime<Utc>) -> Item {
        Item {
            id: crate::sources::parsers::item_id(title),
            kind: ItemKind::Article,
            source: "test".into(),
            title: title.into(),
            snippet: String::new(),
            url: format!("https://example.org/{}", title.len()),
            published: at,
            fetched: at,
            category: cat,
            severity: Severity::Elevated,
            place: None,
            tokens: classify::tokenize(title),
            facts: Vec::new(),
            relevance: 10,
            event_key: None,
            thumbnail: None,
        }
    }

    #[test]
    fn similar_headlines_join_one_story() {
        let now = Utc::now();
        let mut c = Clusterer::default();
        c.ingest(item("Massive earthquake strikes northern Chile coast", Category::Earthquake, now));
        c.ingest(item("Earthquake strikes northern Chile, buildings damaged", Category::Earthquake, now));
        assert_eq!(c.story_count(), 1, "near-identical reports should cluster");
    }

    #[test]
    fn unrelated_headlines_stay_separate() {
        let now = Utc::now();
        let mut c = Clusterer::default();
        c.ingest(item("Earthquake strikes northern Chile coast", Category::Earthquake, now));
        c.ingest(item("Central bank raises interest rates again", Category::Economy, now));
        assert_eq!(c.story_count(), 2);
    }

    #[test]
    fn duplicate_item_is_ignored() {
        let now = Utc::now();
        let mut c = Clusterer::default();
        let it = item("Flooding displaces thousands in Bangladesh", Category::NaturalDisaster, now);
        assert!(c.ingest(it.clone()).is_some());
        assert!(c.ingest(it).is_none(), "same id must not be stored twice");
        assert_eq!(c.item_count(), 1);
    }

    #[test]
    fn anchor_routing_keeps_one_ukraine_timeline() {
        let now = Utc::now();
        let mut c = Clusterer::with_anchors(now);
        for title in [
            "Russian drone strike hits Kharkiv overnight",
            "Zelensky says front line near Pokrovsk is holding",
            "Explosions reported across Odesa as Ukraine repels attack",
        ] {
            let mut it = item(title, Category::UkraineWar, now);
            // The real pipeline attaches this in `finalize`.
            if let Some(a) = classify::match_anchor(title) {
                it.facts.push(("anchor".into(), a.id.clone()));
            }
            c.ingest(it);
        }
        let ukraine = c.stories.get("ukraine-war").expect("anchor exists");
        assert_eq!(ukraine.items.len(), 3, "all three belong to one story");
    }

    #[test]
    fn items_older_than_the_retention_window_are_rejected() {
        let now = Utc::now();
        let mut c = Clusterer::default();
        let old = now - Duration::hours(c.retire_after_hours + 5);
        assert!(c
            .ingest(item("Wildfire burning in remote territory", Category::NaturalDisaster, old))
            .is_none());
        assert_eq!(c.item_count(), 0);
        assert!(c
            .ingest(item("Wildfire burning in remote territory", Category::NaturalDisaster, now))
            .is_some());
    }

    #[test]
    fn distant_disasters_do_not_merge_despite_identical_wording() {
        use crate::model::{GeoPoint, Place};
        let now = Utc::now();
        let mut c = Clusterer::default();
        let mut chile = item("Strong earthquake strikes coastal region", Category::Earthquake, now);
        chile.place = Some(Place {
            name: "Chile".into(),
            point: GeoPoint::new(-35.0, -71.0),
            radius_deg: 2.0,
            bbox: None,
        });
        let mut japan = item("Strong earthquake strikes coastal region today", Category::Earthquake, now);
        japan.place = Some(Place {
            name: "Japan".into(),
            point: GeoPoint::new(36.0, 138.0),
            radius_deg: 2.0,
            bbox: None,
        });
        c.ingest(chile);
        c.ingest(japan);
        assert_eq!(c.story_count(), 2, "same words, opposite sides of the planet");
    }

    #[test]
    fn registry_events_never_merge_with_each_other() {
        // Two different quakes have near-identical titles. Before event keys
        // they collapsed into one story with dozens of unrelated epicentres.
        let now = Utc::now();
        let mut c = Clusterer::default();
        for (title, key) in [
            ("M 4.6 - 75 km SE of Kuqa, China", "usgs-a"),
            ("M 4.7 - 80 km SE of Kuqa, China", "usgs-b"),
        ] {
            let mut it = item(title, Category::Earthquake, now);
            it.event_key = Some(key.to_string());
            c.ingest(it);
        }
        assert_eq!(c.story_count(), 2, "distinct events keep distinct timelines");
    }

    #[test]
    fn updates_to_one_event_join_its_story() {
        let now = Utc::now();
        let mut c = Clusterer::default();
        for title in ["Orange alert for tropical cyclone in Madagascar",
                      "Red alert for tropical cyclone in Madagascar"] {
            let mut it = item(title, Category::NaturalDisaster, now);
            it.event_key = Some("gdacs-TC-1234".to_string());
            c.ingest(it);
        }
        assert_eq!(c.story_count(), 1);
        assert_eq!(c.item_count(), 2, "an upgrade joins the event's own timeline");
    }

    #[test]
    fn anchored_story_pins_to_its_home_location() {
        let now = Utc::now();
        let c = Clusterer::with_anchors(now);
        let focus = c.stories.get("ukraine-war").unwrap().focus();
        assert_eq!(focus.map(|p| p.name).as_deref(), Some("Ukraine"));
    }

    #[test]
    fn stale_stories_retire_but_anchors_persist() {
        let now = Utc::now();
        let mut c = Clusterer::with_anchors(now);
        c.ingest(item("Local bridge closed for repairs somewhere", Category::Other, now));
        let before = c.story_count();

        // Let the clock run past the retention window.
        let later = now + Duration::hours(c.retire_after_hours + 1);
        c.maintain(later);

        assert!(c.story_count() < before, "the quiet story should be retired");
        assert!(
            c.stories.contains_key("ukraine-war"),
            "anchors outlive their quiet periods"
        );
    }

    #[test]
    fn timeline_is_newest_first() {
        let now = Utc::now();
        let mut c = Clusterer::default();
        c.ingest(item("Volcano erupts on Sicily sending ash high", Category::NaturalDisaster, now - Duration::hours(3)));
        c.ingest(item("Volcano erupts on Sicily, flights cancelled", Category::NaturalDisaster, now));
        let story = c.stories.values().next().unwrap();
        assert!(story.items[0].published >= story.items[1].published);
    }
}
