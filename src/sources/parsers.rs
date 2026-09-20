//! Maps each source's wire format onto the common `Item` type, applying
//! classification, geolocation and severity grading on the way through.

use crate::classify;
use crate::geo;
use crate::model::{Category, GeoPoint, Item, ItemKind, Place, Severity};
use crate::sources::feed::{self, Record};
use crate::sources::{Parser, SourceDef, SourceKind};
use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;

/// Stable 64-bit id for an item, so restarts and overlapping feeds dedupe.
pub fn item_id(seed: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in seed.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Everything a parser knows before classification fills in the rest.
struct Draft {
    kind: ItemKind,
    title: String,
    snippet: String,
    url: String,
    published: DateTime<Utc>,
    place: Option<Place>,
    severity: Option<Severity>,
    facts: Vec<(String, String)>,
    /// Registry id when the source tracks discrete physical events.
    event_key: Option<String>,
    /// A picture worth showing, when the source offers one.
    thumbnail: Option<String>,
}

impl Draft {
    fn new(kind: ItemKind, title: impl Into<String>, url: impl Into<String>) -> Self {
        Draft {
            kind,
            title: title.into(),
            snippet: String::new(),
            url: url.into(),
            published: Utc::now(),
            place: None,
            severity: None,
            facts: Vec::new(),
            event_key: None,
            thumbnail: None,
        }
    }

    fn fact(mut self, k: &str, v: impl Into<String>) -> Self {
        let v = v.into();
        if !v.trim().is_empty() {
            self.facts.push((k.to_string(), v));
        }
        self
    }
}

/// Apply the shared pipeline: classify, geolocate, grade, tokenize.
/// Returns `None` when the item is below the relevance floor for its source.
fn finalize(
    draft: Draft,
    def: &SourceDef,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Option<Item> {
    let title = draft.title.trim();
    if title.is_empty() || draft.url.trim().is_empty() {
        return None;
    }
    let text = format!("{} {}", title, draft.snippet);

    // Sport and entertainment borrow crisis vocabulary constantly.
    if draft.event_key.is_none() && classify::is_noise(&text) {
        return None;
    }

    let (guessed, score) = classify::categorize(&text);
    let anchor = classify::match_anchor(&text);

    // Hazard registries and humanitarian reporting are crisis-relevant by
    // construction. Everything else — wires, video channels and social feeds
    // alike — must earn its place, or the dashboard fills with football.
    let gated = !matches!(def.kind, SourceKind::Alert | SourceKind::Humanitarian);
    if gated && anchor.is_none() && score < min_relevance {
        return None;
    }

    let category = def
        .force_category
        .or_else(|| anchor.map(|a| a.category))
        .unwrap_or(guessed);

    let place = draft
        .place
        .clone()
        .or_else(|| geo::locate(&text));

    let severity = draft
        .severity
        .unwrap_or_else(|| classify::severity_from_text(&text, category));

    // Many feeds repeat the headline as the opening of the description; showing
    // both wastes two lines of the timeline on the same sentence.
    let snippet = if is_restatement(title, &draft.snippet) {
        String::new()
    } else {
        draft.snippet
    };

    let mut facts = draft.facts;
    if let Some(toll) = classify::extract_toll(&text) {
        facts.push(("toll".into(), toll));
    }
    if let Some(a) = anchor {
        facts.push(("anchor".into(), a.id.clone()));
    }

    let language = crate::lang::detect_foreign(&text);

    Some(Item {
        id: item_id(&draft.url),
        kind: draft.kind,
        source: def.name.to_string(),
        title: title.to_string(),
        snippet,
        url: draft.url,
        published: draft.published.min(now + chrono::Duration::minutes(5)),
        fetched: now,
        category,
        severity,
        place,
        tokens: classify::tokenize(&text),
        facts,
        relevance: score,
        event_key: draft.event_key,
        thumbnail: draft.thumbnail,
        language,
    })
}

/// Entry point: turn a response body into items.
pub fn build_items(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    match def.parser {
        Parser::Feed => parse_feed(def, body, now, min_relevance),
        Parser::Gdacs => parse_gdacs(def, body, now, min_relevance),
        Parser::UsgsGeoJson => parse_usgs(def, body, now, min_relevance),
        Parser::Eonet => parse_eonet(def, body, now, min_relevance),
        Parser::Gdelt => parse_gdelt(def, body, now, min_relevance),
        Parser::Bluesky => parse_bluesky(def, body, now, min_relevance),
        Parser::Mastodon => parse_mastodon(def, body, now, min_relevance),
        Parser::Telegram => parse_telegram(def, body, now, min_relevance),
    }
}

// ---------------------------------------------------------------- dates

/// Feeds date their items in at least four mutually incompatible ways.
pub fn parse_date(raw: &str) -> Option<DateTime<Utc>> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(d) = DateTime::parse_from_rfc2822(raw) {
        return Some(d.with_timezone(&Utc));
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(raw) {
        return Some(d.with_timezone(&Utc));
    }
    // GDELT: 20260918T061634Z
    if let Ok(d) = NaiveDateTime::parse_from_str(raw, "%Y%m%dT%H%M%SZ") {
        return Some(Utc.from_utc_datetime(&d));
    }
    for fmt in [
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%d %b %Y %H:%M:%S",
        "%a, %d %b %Y %H:%M:%S",
    ] {
        if let Ok(d) = NaiveDateTime::parse_from_str(raw, fmt) {
            return Some(Utc.from_utc_datetime(&d));
        }
    }
    // Some feeds emit RFC2822 with a bare zone name chrono rejects.
    if let Some(cut) = raw.rfind(' ') {
        if let Ok(d) = DateTime::parse_from_rfc2822(&format!("{} +0000", &raw[..cut])) {
            return Some(d.with_timezone(&Utc));
        }
    }
    None
}

fn record_date(rec: &Record, now: DateTime<Utc>) -> DateTime<Utc> {
    rec.first(&["pubdate", "published", "updated", "date", "dc:date", "dateadded"])
        .and_then(parse_date)
        .unwrap_or(now)
}

fn record_link(rec: &Record) -> Option<String> {
    // RSS puts the URL in <link>'s text; Atom puts it in a href attribute.
    rec.first(&["link", "link@href", "guid", "id"])
        .map(|s| s.trim().to_string())
        .filter(|s| s.starts_with("http"))
}

/// A picture worth showing, if the record carries one. Checked in order of
/// how likely each tag is to actually be a photo rather than an icon:
/// `media:thumbnail` (YouTube, many news feeds), a YouTube video id (every
/// public video has a predictable thumbnail URL, tag or no tag), `media:content`
/// tagged as an image, then a generic `<enclosure>` — GDACS attaches a hazard
/// map this way, and it uses the same parser as everything else here.
fn extract_thumbnail(rec: &Record, video_id: Option<&str>) -> Option<String> {
    if let Some(u) = rec.get("thumbnail@url") {
        return Some(u.to_string());
    }
    if let Some(id) = video_id {
        return Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"));
    }
    if let Some(u) = rec.get("content@url") {
        let is_image = rec.get("content@medium").is_none_or(|m| m.eq_ignore_ascii_case("image"));
        if is_image {
            return Some(u.to_string());
        }
    }
    if let Some(u) = rec.get("enclosure@url") {
        let is_image = rec.get("enclosure@type").is_none_or(|t| t.starts_with("image"));
        if is_image {
            return Some(u.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------- RSS / Atom / YouTube

fn parse_feed(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    let records = feed::parse(body)?;
    let mut out = Vec::new();
    for rec in records {
        let title = feed::clean_text(rec.get("title").unwrap_or_default());
        let url = match record_link(&rec) {
            Some(u) => u,
            None => continue,
        };
        let is_video = def.kind == SourceKind::Video || url.contains("youtube.com/watch");
        let kind = if is_video {
            ItemKind::Video
        } else if def.kind == SourceKind::Humanitarian {
            ItemKind::Report
        } else {
            ItemKind::Article
        };

        let snippet = rec
            .first(&["description", "summary", "content", "media:description", "encoded"])
            .map(feed::clean_text)
            .unwrap_or_default();

        let mut draft = Draft::new(kind, title, url);
        draft.snippet = truncate_snippet(&snippet, 420);
        draft.published = record_date(&rec, now);
        draft.thumbnail = extract_thumbnail(&rec, is_video.then(|| rec.get("videoid")).flatten());

        if is_video {
            let channel = rec
                .first(&["author.name", "name", "source@url"])
                .unwrap_or(def.name);
            draft = draft.fact("channel", channel);
            if let Some(views) = rec.first(&["statistics@views"]) {
                draft = draft.fact("views", views);
            }
        }
        if let Some(cat) = rec.get("category") {
            draft = draft.fact("feed-category", feed::clean_text(cat));
        }

        if let Some(item) = finalize(draft, def, now, min_relevance) {
            out.push(item);
        }
    }
    Ok(out)
}

fn truncate_snippet(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    match cut.rfind(' ') {
        Some(i) => format!("{}…", &cut[..i]),
        None => format!("{cut}…"),
    }
}

// ---------------------------------------------------------------- GDACS

/// GDACS grades every alert itself, which is more trustworthy than reading the
/// headline, and gives exact coordinates plus an affected-area bounding box.
fn parse_gdacs(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    let records = feed::parse(body)?;
    let mut out = Vec::new();
    for rec in records {
        let title = feed::clean_text(rec.get("title").unwrap_or_default());
        let url = match record_link(&rec) {
            Some(u) => u,
            None => continue,
        };
        let alert = rec.get("alertlevel").unwrap_or("Green");
        let severity = match alert.to_ascii_lowercase().as_str() {
            "red" => Severity::Critical,
            "orange" => Severity::Severe,
            "green" => Severity::Watch,
            _ => Severity::Info,
        };
        // GDACS publishes a rolling archive of several hundred alerts, the vast
        // majority green (routine, no action). Only graded alerts are events.
        if severity < Severity::Severe {
            continue;
        }

        let event_type = rec.get("eventtype").unwrap_or("");
        let category = match event_type {
            "EQ" => Category::Earthquake,
            "TC" | "FL" | "VO" | "DR" | "WF" => Category::NaturalDisaster,
            _ => Category::NaturalDisaster,
        };

        let place = gdacs_place(&rec, &title);
        let mut draft = Draft::new(ItemKind::Alert, title, url);
        draft.snippet = truncate_snippet(
            &feed::clean_text(rec.get("description").unwrap_or_default()),
            420,
        );
        draft.published = record_date(&rec, now);
        draft.place = place;
        draft.severity = Some(severity);
        draft.thumbnail = extract_thumbnail(&rec, None);
        draft.event_key = rec
            .get("eventid")
            .map(|id| format!("gdacs-{event_type}-{id}"));
        draft = draft
            .fact("alert", alert)
            .fact("event", gdacs_event_label(event_type));
        if let Some(sev) = rec.get("severity") {
            draft = draft.fact("scale", feed::clean_text(sev));
        }
        if let Some(pop) = rec.get("population") {
            draft = draft.fact("population", feed::clean_text(pop));
        }
        if let Some(country) = rec.get("country") {
            draft = draft.fact("country", country);
        }

        let mut item = match finalize(draft, def, now, min_relevance) {
            Some(i) => i,
            None => continue,
        };
        item.category = category;
        out.push(item);
    }
    Ok(out)
}

fn gdacs_event_label(code: &str) -> &'static str {
    match code {
        "EQ" => "earthquake",
        "TC" => "tropical cyclone",
        "FL" => "flood",
        "VO" => "volcano",
        "DR" => "drought",
        "WF" => "wildfire",
        "TS" => "tsunami",
        _ => "hazard",
    }
}

fn gdacs_place(rec: &Record, title: &str) -> Option<Place> {
    let lat: f64 = rec.first(&["lat", "point.lat"])?.parse().ok()?;
    let lon: f64 = rec.first(&["long", "point.long"])?.parse().ok()?;
    let point = GeoPoint::new(lat, lon);
    // GDACS titles read "Red alert for tropical cyclone in Madagascar"; the
    // country name is the most useful label, so prefer it over reverse lookup.
    let name = rec
        .get("country")
        .map(|c| c.split(',').next().unwrap_or(c).trim().to_string())
        .or_else(|| geo::locate(title).map(|p| p.name))
        .or_else(|| geo::nearest(point).map(|p| p.name))
        .unwrap_or_else(|| format!("{lat:.1}, {lon:.1}"));
    // The bbox tells us exactly how wide the affected area is, and — unlike
    // every other source, which only ever gets a guessed radius — lets the
    // map draw the real shape of it rather than an approximating circle.
    let parsed_bbox: Option<[f64; 4]> = rec.get("bbox").and_then(|b| {
        let v: Vec<f64> = b.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        (v.len() == 4).then(|| [v[0], v[1], v[2], v[3]])
    });
    let radius = parsed_bbox
        .map(|[lon_min, lon_max, lat_min, lat_max]| {
            (lon_max - lon_min).abs().max((lat_max - lat_min).abs()) / 2.0
        })
        .unwrap_or(1.5)
        .clamp(0.3, 12.0);
    Some(Place { name, point, radius_deg: radius, bbox: parsed_bbox })
}

// ---------------------------------------------------------------- USGS

fn parse_usgs(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    let v: Value = serde_json::from_str(body)?;
    let features = v["features"].as_array().ok_or_else(|| anyhow!("no features"))?;
    let mut out = Vec::new();
    for f in features {
        let props = &f["properties"];
        let coords = f["geometry"]["coordinates"].as_array();
        let title = props["title"].as_str().unwrap_or("").to_string();
        let url = props["url"].as_str().unwrap_or("").to_string();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let mag = props["mag"].as_f64().unwrap_or(0.0);
        let tsunami = props["tsunami"].as_i64().unwrap_or(0) == 1;
        let severity = if tsunami || mag >= 7.0 {
            Severity::Critical
        } else if mag >= 6.0 {
            Severity::Severe
        } else if mag >= 5.0 {
            Severity::Elevated
        } else {
            Severity::Watch
        };

        let place = coords.and_then(|c| {
            let lon = c.first()?.as_f64()?;
            let lat = c.get(1)?.as_f64()?;
            let point = GeoPoint::new(lat, lon);
            // USGS place strings read "25 km SSE of Kandrian, Papua New Guinea";
            // the region after the last comma is the useful label.
            let name = props["place"]
                .as_str()
                .map(|p| p.rsplit(',').next().unwrap_or(p).trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| geo::nearest(point).map(|p| p.name))
                .unwrap_or_else(|| format!("{lat:.1}, {lon:.1}"));
            Some(Place { name, point, radius_deg: (mag / 3.0).clamp(0.4, 4.0), bbox: None })
        });

        let depth = coords.and_then(|c| c.get(2)?.as_f64()).unwrap_or(0.0);
        let mut draft = Draft::new(ItemKind::Alert, &title, &url);
        draft.event_key = f["id"].as_str().map(|id| format!("usgs-{id}"));
        draft.published = props["time"]
            .as_i64()
            .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
            .unwrap_or(now);
        draft.place = place;
        draft.severity = Some(severity);
        draft.snippet = format!(
            "Magnitude {:.1} earthquake at {:.0} km depth{}.",
            mag,
            depth,
            if tsunami { ", tsunami evaluation issued" } else { "" }
        );
        draft = draft
            .fact("magnitude", format!("{mag:.1}"))
            .fact("depth", format!("{depth:.0} km"));
        if tsunami {
            draft = draft.fact("tsunami", "yes");
        }
        if let Some(item) = finalize(draft, def, now, min_relevance) {
            out.push(item);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- NASA EONET

fn parse_eonet(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    let v: Value = serde_json::from_str(body)?;
    let events = v["events"].as_array().ok_or_else(|| anyhow!("no events"))?;
    let mut out = Vec::new();
    for e in events {
        let title = e["title"].as_str().unwrap_or("").to_string();
        let url = e["link"]
            .as_str()
            .map(|s| s.to_string())
            .or_else(|| e["id"].as_str().map(|id| format!("https://eonet.gsfc.nasa.gov/api/v3/events/{id}")))
            .unwrap_or_default();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let category_title = e["categories"][0]["title"].as_str().unwrap_or("Event");
        // The last geometry entry is the most recent observation of the event.
        let geometry = e["geometry"].as_array().and_then(|g| g.last());
        let place = geometry.and_then(|g| {
            let c = g["coordinates"].as_array()?;
            let lon = c.first()?.as_f64()?;
            let lat = c.get(1)?.as_f64()?;
            let point = GeoPoint::new(lat, lon);
            let name = geo::nearest(point).map(|p| p.name).unwrap_or_else(|| format!("{lat:.1}, {lon:.1}"));
            Some(Place { name, point, radius_deg: 2.0, bbox: None })
        });
        let published = geometry
            .and_then(|g| g["date"].as_str())
            .and_then(parse_date)
            .unwrap_or(now);

        let mut draft = Draft::new(ItemKind::Alert, &title, &url);
        draft.event_key = e["id"].as_str().map(|id| format!("eonet-{id}"));
        draft.published = published;
        draft.place = place;
        draft.snippet = format!("{category_title} tracked by NASA EONET.");
        draft.severity = Some(Severity::Elevated);
        draft = draft.fact("event", category_title.to_lowercase());
        if let Some(item) = finalize(draft, def, now, min_relevance) {
            out.push(item);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- GDELT

fn parse_gdelt(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    // GDELT answers rate-limit violations with prose, not JSON.
    if !body.trim_start().starts_with('{') {
        return Err(anyhow!("rate limited"));
    }
    let v: Value = serde_json::from_str(body)?;
    let articles = v["articles"].as_array().ok_or_else(|| anyhow!("no articles"))?;
    let mut out = Vec::new();
    for a in articles {
        let title = a["title"].as_str().unwrap_or("").to_string();
        let url = a["url"].as_str().unwrap_or("").to_string();
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let mut draft = Draft::new(ItemKind::Article, &title, &url);
        draft.published = a["seendate"].as_str().and_then(parse_date).unwrap_or(now);
        if let Some(domain) = a["domain"].as_str() {
            draft = draft.fact("outlet", domain);
        }
        if let Some(country) = a["sourcecountry"].as_str().filter(|c| !c.is_empty()) {
            draft.place = geo::lookup(country);
            draft = draft.fact("source country", country);
        }
        if let Some(item) = finalize(draft, def, now, min_relevance) {
            out.push(item);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- Bluesky (AT Protocol)

/// `app.bsky.feed.getAuthorFeed` on the public AppView: no key, no auth, and
/// posts land seconds after they are written.
fn parse_bluesky(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    let v: Value = serde_json::from_str(body)?;
    let feed = v["feed"].as_array().ok_or_else(|| anyhow!("no feed"))?;
    let mut out = Vec::new();
    for entry in feed {
        let post = &entry["post"];
        // Reposts carry a `reason`; they duplicate content we already have.
        if !entry["reason"].is_null() {
            continue;
        }
        let record = &post["record"];
        let text = record["text"].as_str().unwrap_or("").trim().to_string();
        if text.is_empty() {
            continue;
        }
        let handle = post["author"]["handle"].as_str().unwrap_or("unknown");
        let uri = post["uri"].as_str().unwrap_or("");
        // at://did/app.bsky.feed.post/rkey -> the web permalink
        let rkey = uri.rsplit('/').next().unwrap_or("");
        let web_url = format!("https://bsky.app/profile/{handle}/post/{rkey}");

        // A post linking to an article is best represented by the article itself.
        let external = post["embed"]["external"]["uri"].as_str();
        let url = external.unwrap_or(&web_url).to_string();
        // A directly-attached photo is worth more than a link card's preview.
        let thumbnail = post["embed"]["images"][0]["thumb"]
            .as_str()
            .or_else(|| post["embed"]["external"]["thumb"].as_str())
            .map(|s| s.to_string());

        let (title, snippet) = split_social_text(&text);
        if title.chars().count() < 25 {
            continue; // A bare link or a two-word post is not a report.
        }
        let mut draft = Draft::new(ItemKind::Article, title, url);
        draft.snippet = snippet;
        draft.thumbnail = thumbnail;
        draft.published = post["indexedAt"]
            .as_str()
            .or_else(|| record["createdAt"].as_str())
            .and_then(parse_date)
            .unwrap_or(now);
        draft = draft.fact("handle", format!("@{handle}")).fact("post", web_url);
        if let Some(n) = post["replyCount"].as_i64() {
            draft = draft.fact("replies", n.to_string());
        }
        if let Some(item) = finalize(draft, def, now, min_relevance) {
            out.push(item);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- Mastodon

/// Public hashtag timelines are unauthenticated on every standard instance.
fn parse_mastodon(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    let v: Value = serde_json::from_str(body)?;
    let posts = v.as_array().ok_or_else(|| anyhow!("not an array"))?;
    let mut out = Vec::new();
    for p in posts {
        if !p["reblog"].is_null() {
            continue;
        }
        let content = feed::clean_text(p["content"].as_str().unwrap_or(""));
        if content.trim().is_empty() {
            continue;
        }
        let acct = p["account"]["acct"].as_str().unwrap_or("unknown");
        let post_url = p["url"].as_str().unwrap_or("").to_string();
        if post_url.is_empty() {
            continue;
        }
        // Prefer a linked article card over the toot itself.
        let url = p["card"]["url"].as_str().unwrap_or(&post_url).to_string();
        let thumbnail = p["media_attachments"][0]["preview_url"]
            .as_str()
            .or_else(|| p["media_attachments"][0]["url"].as_str())
            .or_else(|| p["card"]["image"].as_str())
            .map(|s| s.to_string());

        let (title, snippet) = split_social_text(&content);
        if title.chars().count() < 25 {
            continue;
        }
        let mut draft = Draft::new(ItemKind::Article, title, url);
        draft.snippet = snippet;
        draft.thumbnail = thumbnail;
        draft.published = p["created_at"].as_str().and_then(parse_date).unwrap_or(now);
        draft = draft.fact("account", format!("@{acct}")).fact("post", post_url);
        if let Some(item) = finalize(draft, def, now, min_relevance) {
            out.push(item);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- Telegram

/// Public channels expose a server-rendered preview at `t.me/s/<channel>`.
/// There is no API and no key; this reads the same HTML a browser would.
fn parse_telegram(
    def: &SourceDef,
    body: &str,
    now: DateTime<Utc>,
    min_relevance: u32,
) -> Result<Vec<Item>> {
    let channel = def
        .url
        .rsplit('/')
        .next()
        .unwrap_or("channel")
        .to_string();
    let mut out = Vec::new();

    // Each message is a <div class="tgme_widget_message ..." data-post="chan/123">
    // containing a text div and a <time datetime="...">.
    for block in body.split("tgme_widget_message_wrap").skip(1) {
        let post_id = extract_attr(block, "data-post=\"");
        let text_html = extract_between(block, "js-message_text", "</div>");
        let Some(text_html) = text_html else { continue };
        let text = feed::clean_text(text_html);
        if text.trim().len() < 12 {
            continue;
        }
        let published = extract_attr(block, "datetime=\"")
            .and_then(|d| parse_date(&d))
            .unwrap_or(now);
        let url = post_id
            .map(|p| format!("https://t.me/{p}"))
            .unwrap_or_else(|| format!("https://t.me/s/{channel}"));

        let (title, snippet) = split_social_text(&text);
        if title.chars().count() < 25 {
            continue;
        }
        let mut draft = Draft::new(ItemKind::Article, title, url);
        draft.snippet = snippet;
        draft.published = published;
        draft = draft.fact("channel", format!("t.me/{channel}"));
        if block.contains("tgme_widget_message_video") {
            draft.kind = ItemKind::Video;
        }
        if let Some(item) = finalize(draft, def, now, min_relevance) {
            out.push(item);
        }
    }
    Ok(out)
}

fn extract_attr(haystack: &str, prefix: &str) -> Option<String> {
    let at = haystack.find(prefix)? + prefix.len();
    let rest = &haystack[at..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_between<'a>(haystack: &'a str, after: &str, until: &str) -> Option<&'a str> {
    let at = haystack.find(after)? + after.len();
    let rest = &haystack[at..];
    // Skip the remainder of the opening tag.
    let start = rest.find('>')? + 1;
    let rest = &rest[start..];
    let end = rest.find(until).unwrap_or(rest.len());
    Some(&rest[..end])
}

/// True when the snippet merely repeats the headline.
fn is_restatement(title: &str, snippet: &str) -> bool {
    let snippet = snippet.trim();
    if snippet.is_empty() {
        return true;
    }
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric() || c.is_whitespace())
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let (t, s) = (norm(title), norm(snippet));
    if t.is_empty() {
        return false;
    }
    // The description opens with the headline, or is a truncation of it.
    s.starts_with(&t) || t.starts_with(&s)
}

/// Social posts arrive wrapped in decoration: leading emoji, bare URLs,
/// trailing hashtag walls. Strip all of it before anything else looks at the
/// text, or headlines become "🔗 https://…".
fn tidy_social(text: &str) -> String {
    let mut kept: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        // Bare links carry no information in a headline; the item has a url.
        if word.starts_with("http://") || word.starts_with("https://") || word.starts_with("www.") {
            continue;
        }
        let trimmed: String = word
            .chars()
            .filter(|c| {
                // Keep letters, digits, punctuation and spaces; drop pictographs.
                c.is_alphanumeric()
                    || c.is_ascii_punctuation()
                    || c.is_whitespace()
                    || matches!(c, '\u{00C0}'..='\u{024F}' | '\u{0400}'..='\u{04FF}' | '…' | '—' | '–' | '\u{2018}'..='\u{201F}')
            })
            .collect();
        let trimmed = trimmed.trim().to_string();
        if trimmed.is_empty() || trimmed == "#" || trimmed == "-" || trimmed == "|" {
            continue;
        }
        kept.push(trimmed);
    }
    // Drop a trailing run of hashtags — they are tags, not a sentence.
    while kept.last().is_some_and(|w| w.starts_with('#')) {
        kept.pop();
    }
    kept.join(" ").trim().to_string()
}

/// Social posts have no title/body split; use the first sentence as the headline
/// so list rows stay readable and the rest becomes the snippet.
fn split_social_text(text: &str) -> (String, String) {
    let clean = tidy_social(text);
    // The *first* sentence break past a useful minimum length — taking the last
    // one would swallow the whole post into the headline.
    let cut = clean
        .char_indices()
        .take_while(|(i, _)| *i < 160)
        .filter(|(_, c)| matches!(c, '.' | '!' | '?' | '\n'))
        .map(|(i, _)| i + 1)
        .find(|i| *i > 40);
    match cut {
        Some(i) if i > 40 && i < clean.len() => {
            (clean[..i].trim().to_string(), clean[i..].trim().to_string())
        }
        _ if clean.chars().count() > 160 => {
            let head: String = clean.chars().take(160).collect();
            let at = head.rfind(' ').unwrap_or(head.len());
            (format!("{}…", &head[..at]), clean[at..].trim().to_string())
        }
        _ => (clean, String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_prefers_explicit_tag_over_video_id_fallback() {
        let xml = r#"<item>
            <title>t</title>
            <link>https://example.org/1</link>
            <media:thumbnail url="https://example.org/explicit.jpg"/>
            <yt:videoId>abc123</yt:videoId>
        </item>"#;
        let rec = &feed::parse(xml).unwrap()[0];
        assert_eq!(
            extract_thumbnail(rec, rec.get("videoid")),
            Some("https://example.org/explicit.jpg".to_string())
        );
    }

    #[test]
    fn thumbnail_falls_back_to_youtube_video_id() {
        let xml = r#"<item>
            <title>t</title>
            <link>https://example.org/1</link>
            <yt:videoId>abc123</yt:videoId>
        </item>"#;
        let rec = &feed::parse(xml).unwrap()[0];
        assert_eq!(
            extract_thumbnail(rec, rec.get("videoid")),
            Some("https://i.ytimg.com/vi/abc123/hqdefault.jpg".to_string())
        );
    }

    #[test]
    fn thumbnail_ignores_non_image_enclosures() {
        let xml = r#"<item>
            <title>t</title>
            <link>https://example.org/1</link>
            <enclosure url="https://example.org/podcast.mp3" type="audio/mpeg"/>
        </item>"#;
        let rec = &feed::parse(xml).unwrap()[0];
        assert_eq!(extract_thumbnail(rec, None), None);
    }

    #[test]
    fn thumbnail_accepts_gdacs_style_image_enclosure() {
        let xml = r#"<item>
            <title>t</title>
            <link>https://example.org/1</link>
            <enclosure type="image/png" url="https://example.org/hazard.png"/>
        </item>"#;
        let rec = &feed::parse(xml).unwrap()[0];
        assert_eq!(
            extract_thumbnail(rec, None),
            Some("https://example.org/hazard.png".to_string())
        );
    }

    #[test]
    fn parses_gdelt_dates() {
        assert!(parse_date("20260918T061634Z").is_some());
        assert!(parse_date("Fri, 18 Sep 2026 06:16:34 GMT").is_some());
        assert!(parse_date("2026-09-18T06:16:34+00:00").is_some());
        assert!(parse_date("").is_none());
    }

    #[test]
    fn splits_social_text_at_sentence() {
        let (t, s) = split_social_text("Explosions reported in the capital overnight. Residents describe a long barrage.");
        assert_eq!(t, "Explosions reported in the capital overnight.");
        assert!(s.starts_with("Residents"));
    }

    #[test]
    fn short_post_has_no_snippet() {
        let (t, s) = split_social_text("Breaking news");
        assert_eq!(t, "Breaking news");
        assert!(s.is_empty());
    }

    #[test]
    fn snippet_repeating_the_headline_is_dropped() {
        assert!(is_restatement("Flooding hits Nepal", "Flooding hits Nepal, officials say"));
        assert!(is_restatement("Flooding hits Nepal", "Flooding hits Nep"));
        assert!(is_restatement("Flooding hits Nepal", "   "));
        assert!(!is_restatement(
            "Flooding hits Nepal",
            "Rescue teams reached the valley after two days"
        ));
    }

    #[test]
    fn item_id_is_stable_and_distinct() {
        assert_eq!(item_id("https://a"), item_id("https://a"));
        assert_ne!(item_id("https://a"), item_id("https://b"));
    }
}
