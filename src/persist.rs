//! Saving and restoring the accumulated timeline, so a restart does not lose
//! hours of context.

use crate::model::Story;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Bumped whenever the on-disk shape changes; older files are ignored rather
/// than mis-parsed.
const FORMAT_VERSION: u32 = 1;

/// Stories older than this are not written back out. Shared with the
/// clusterer and the story list so "no update in 2 months" means the same
/// thing everywhere.
const MAX_AGE_HOURS: i64 = crate::model::RETENTION_HOURS;
const MAX_STORIES: usize = 400;
const MAX_ITEMS_PER_STORY: usize = 250;

#[derive(Serialize, Deserialize)]
struct Snapshot {
    version: u32,
    saved_at: chrono::DateTime<chrono::Utc>,
    stories: HashMap<String, Story>,
}

pub fn state_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("crisis-dash")
        .join("state.json")
}

pub fn save(stories: &HashMap<String, Story>) -> Result<()> {
    let now = chrono::Utc::now();
    let mut keep: Vec<(&String, &Story)> = stories
        .iter()
        .filter(|(_, s)| !s.items.is_empty() && !s.is_stale(now, MAX_AGE_HOURS))
        .collect();
    // Keep the most recently active stories when over budget.
    keep.sort_by(|a, b| b.1.last_update.cmp(&a.1.last_update));
    keep.truncate(MAX_STORIES);

    let trimmed: HashMap<String, Story> = keep
        .into_iter()
        .map(|(id, story)| {
            let mut s = story.clone();
            s.trim(MAX_ITEMS_PER_STORY);
            (id.clone(), s)
        })
        .collect();

    let snapshot = Snapshot { version: FORMAT_VERSION, saved_at: now, stories: trimmed };
    let path = state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }

    // Write to a temporary file and rename, so an interrupted save cannot leave
    // a truncated state file behind.
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(&snapshot)?;
    std::fs::write(&tmp, json).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

pub fn load() -> Result<Option<HashMap<String, Story>>> {
    let path = state_path();
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let snapshot: Snapshot = match serde_json::from_slice(&raw) {
        Ok(s) => s,
        // A state file from an older layout is not worth migrating; the
        // dashboard refills within minutes.
        Err(e) => {
            let _ = std::fs::rename(&path, path.with_extension("json.bad"));
            return Err(anyhow::anyhow!("state file unreadable ({e}); moved aside"));
        }
    };
    if snapshot.version != FORMAT_VERSION {
        return Ok(None);
    }
    let now = chrono::Utc::now();
    let stories: HashMap<String, Story> = snapshot
        .stories
        .into_iter()
        .filter(|(_, s)| !s.is_stale(now, MAX_AGE_HOURS))
        .collect();
    Ok(Some(stories))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Category, Item, ItemKind, Severity};

    fn story_with_item(id: &str, at: chrono::DateTime<chrono::Utc>) -> Story {
        let item = Item {
            id: format!("{id}-item"),
            kind: ItemKind::Article,
            source: "test".into(),
            title: "A headline".into(),
            snippet: String::new(),
            url: "https://example.org".into(),
            published: at,
            fetched: at,
            category: Category::Other,
            severity: Severity::Info,
            place: None,
            tokens: vec!["headline".into()],
            facts: Vec::new(),
            relevance: 5,
            event_key: None,
            thumbnail: None,
        };
        Story::new(id.to_string(), &item)
    }

    #[test]
    fn snapshot_round_trips_through_json() {
        let now = chrono::Utc::now();
        let mut stories = HashMap::new();
        stories.insert("s1".to_string(), story_with_item("s1", now));

        let snap = Snapshot { version: FORMAT_VERSION, saved_at: now, stories };
        let json = serde_json::to_vec(&snap).unwrap();
        let back: Snapshot = serde_json::from_slice(&json).unwrap();

        assert_eq!(back.version, FORMAT_VERSION);
        assert_eq!(back.stories.len(), 1);
        assert_eq!(back.stories["s1"].items.len(), 1);
        assert!(back.stories["s1"].seen_ids.contains("s1-item"));
    }
}
