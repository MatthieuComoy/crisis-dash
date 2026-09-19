//! Source registry and the scrapers that turn each source into `Item`s.

pub mod feed;
pub mod parsers;

use crate::model::Category;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SourceKind {
    /// General international news wire.
    News,
    /// A news organisation's YouTube channel.
    Video,
    /// Machine-generated hazard alert with authoritative coordinates.
    Alert,
    /// Humanitarian situation reporting.
    Humanitarian,
    /// Social platform posts — fastest signal, lowest reliability.
    Social,
}

impl SourceKind {
    pub fn label(&self) -> &'static str {
        match self {
            SourceKind::News => "news",
            SourceKind::Video => "video",
            SourceKind::Alert => "alert",
            SourceKind::Humanitarian => "humanitarian",
            SourceKind::Social => "social",
        }
    }
}

/// How the response body is turned into records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Parser {
    /// RSS / RDF / Atom, including YouTube channel feeds.
    Feed,
    /// GDACS RSS, which carries alert level, event type and coordinates.
    Gdacs,
    /// USGS earthquake GeoJSON.
    UsgsGeoJson,
    /// NASA EONET v3 JSON.
    Eonet,
    /// GDELT 2.0 DOC API article list (JSON).
    Gdelt,
    /// Bluesky AT Protocol public AppView (`getAuthorFeed`).
    Bluesky,
    /// Mastodon public hashtag timeline.
    Mastodon,
    /// Telegram public channel preview page (HTML).
    Telegram,
}

#[derive(Debug, Clone)]
pub struct SourceDef {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: SourceKind,
    pub url: &'static str,
    pub parser: Parser,
    /// Seconds between polls. Wires move faster than disaster registries.
    pub interval: u64,
    /// Forced category for sources that only ever report one kind of event.
    pub force_category: Option<Category>,
    /// Off by default: the source works but rate-limits aggressively.
    pub opt_in: bool,
}

const fn s(
    id: &'static str,
    name: &'static str,
    kind: SourceKind,
    url: &'static str,
    parser: Parser,
    interval: u64,
    force_category: Option<Category>,
    opt_in: bool,
) -> SourceDef {
    SourceDef { id, name, kind, url, parser, interval, force_category, opt_in }
}

/// Every source shipped with the dashboard. All are free and keyless; the
/// YouTube channel feeds are the public per-channel Atom endpoints, which need
/// no Data API quota.
pub fn registry() -> Vec<SourceDef> {
    use Category::*;
    use Parser::*;
    use SourceKind::*;
    vec![
        // --- Hazard and humanitarian registries: authoritative coordinates ---
        s("gdacs", "GDACS alerts", Alert, "https://www.gdacs.org/xml/rss.xml", Gdacs, 300, None, false),
        s("usgs", "USGS earthquakes", Alert,
          "https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/4.5_day.geojson",
          UsgsGeoJson, 180, Some(Earthquake), false),
        s("eonet", "NASA EONET", Alert,
          "https://eonet.gsfc.nasa.gov/api/v3/events?status=open&limit=60",
          Eonet, 900, Some(NaturalDisaster), false),
        s("reliefweb", "ReliefWeb disasters", SourceKind::Humanitarian,
          "https://reliefweb.int/disasters/rss.xml", Feed, 900, None, false),
        s("un-news", "UN News", SourceKind::Humanitarian,
          "https://news.un.org/feed/subscribe/en/news/all/rss.xml", Feed, 600, None, false),

        // --- International news wires ---
        s("bbc", "BBC World", News, "https://feeds.bbci.co.uk/news/world/rss.xml", Feed, 240, None, false),
        s("aljazeera", "Al Jazeera", News, "https://www.aljazeera.com/xml/rss/all.xml", Feed, 240, None, false),
        s("guardian", "Guardian World", News, "https://www.theguardian.com/world/rss", Feed, 240, None, false),
        s("france24", "France 24", News, "https://www.france24.com/en/rss", Feed, 300, None, false),
        s("dw", "Deutsche Welle", News, "https://rss.dw.com/rdf/rss-en-world", Feed, 300, None, false),
        s("npr", "NPR World", News, "https://feeds.npr.org/1004/rss.xml", Feed, 420, None, false),
        s("cbs", "CBS World", News, "https://www.cbsnews.com/latest/rss/world", Feed, 300, None, false),
        s("wsj", "WSJ World", News, "https://feeds.a.dj.com/rss/RSSWorldNews.xml", Feed, 420, None, false),
        s("scmp", "South China Morning Post", News, "https://www.scmp.com/rss/91/feed", Feed, 420, None, false),
        s("toi", "Times of Israel", News, "https://www.timesofisrael.com/feed/", Feed, 300, None, false),
        s("elpais", "El País (English)", News,
          "https://feeds.elpais.com/mrss-s/pages/ep/site/english.elpais.com/portada", Feed, 420, None, false),
        s("fox", "Fox World", News, "https://moxie.foxnews.com/google-publisher/world.xml", Feed, 420, None, false),

        // --- YouTube channel feeds (public Atom, no API key) ---
        s("yt-bbc", "YouTube · BBC News", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UC16niRr50-MSBwiO3YDb3RA", Feed, 600, None, false),
        s("yt-aljazeera", "YouTube · Al Jazeera", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UCNye-wNBqNL5ZzHSJj3l8Bg", Feed, 600, None, false),
        s("yt-dw", "YouTube · DW News", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UCknLrEdhRCp1aegoMqRaCZg", Feed, 600, None, false),
        s("yt-france24", "YouTube · France 24", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UCQfwfsi5VrQ8yKZ-UWmAEFg", Feed, 600, None, false),
        s("yt-reuters", "YouTube · Reuters", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UChqUTb7kYRX8-EiaN3XFrSQ", Feed, 600, None, false),
        s("yt-sky", "YouTube · Sky News", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UCoMdktPbSTixAyNGwb-UYkQ", Feed, 600, None, false),
        s("yt-guardian", "YouTube · Guardian", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UCIRYBXDze5krPDzAEOxFGVA", Feed, 600, None, false),
        s("yt-ap", "YouTube · AP", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UC52X5wxOL_s5yw0dQk7NtgA", Feed, 600, None, false),
        s("yt-cna", "YouTube · CNA", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UC83jt4dlz1Gjl58fzQrrKZg", Feed, 900, None, false),
        s("yt-nbc", "YouTube · NBC News", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UCeY0bbntWzzVIaj2z3QigXg", Feed, 900, None, false),
        s("yt-wion", "YouTube · WION", Video,
          "https://www.youtube.com/feeds/videos.xml?channel_id=UC_gUM8rL-Lrg6O3adPW9K1g", Feed, 900, None, false),


        // --- Social: the fastest signal available without a paid API key ---
        // Bluesky's public AppView needs no token; these are the wire services'
        // own accounts, so posts arrive ahead of their RSS items.
        s("bsky-reuters", "Bluesky · Reuters", Social,
          "https://public.api.bsky.app/xrpc/app.bsky.feed.getAuthorFeed?actor=reuters.com&limit=40&filter=posts_no_replies",
          Bluesky, 120, None, false),
        s("bsky-ap", "Bluesky · AP", Social,
          "https://public.api.bsky.app/xrpc/app.bsky.feed.getAuthorFeed?actor=apnews.com&limit=40&filter=posts_no_replies",
          Bluesky, 120, None, false),
        s("bsky-guardian", "Bluesky · Guardian", Social,
          "https://public.api.bsky.app/xrpc/app.bsky.feed.getAuthorFeed?actor=theguardian.com&limit=40&filter=posts_no_replies",
          Bluesky, 180, None, false),
        s("bsky-aljazeera", "Bluesky · Al Jazeera", Social,
          "https://public.api.bsky.app/xrpc/app.bsky.feed.getAuthorFeed?actor=aljazeera.com&limit=40&filter=posts_no_replies",
          Bluesky, 180, None, false),
        s("bsky-npr", "Bluesky · NPR", Social,
          "https://public.api.bsky.app/xrpc/app.bsky.feed.getAuthorFeed?actor=npr.org&limit=40&filter=posts_no_replies",
          Bluesky, 240, None, false),

        // Mastodon hashtag timelines: unauthenticated, and where a lot of
        // on-the-ground reporting now lands.
        s("masto-breaking", "Mastodon · #breakingnews", Social,
          "https://mastodon.social/api/v1/timelines/tag/breakingnews?limit=40", Mastodon, 180, None, false),
        s("masto-ukraine", "Mastodon · #ukraine", Social,
          "https://mastodon.social/api/v1/timelines/tag/ukraine?limit=40", Mastodon, 240, None, false),
        s("masto-gaza", "Mastodon · #gaza", Social,
          "https://mastodon.social/api/v1/timelines/tag/gaza?limit=40", Mastodon, 240, None, false),
        // Note: the #earthquake tag is almost entirely micro-seismicity bots
        // that duplicate USGS with worse metadata, so it is not included.
        s("masto-conflict", "Mastodon · #conflict", Social,
          "https://mastodon.social/api/v1/timelines/tag/conflict?limit=40", Mastodon, 300, None, false),

        // Reddit serves RSS without a key, but rate-limits datacentre IPs hard,
        // so it is opt-in and polled slowly.
        s("reddit-worldnews", "Reddit · r/worldnews", Social,
          "https://www.reddit.com/r/worldnews/new/.rss?limit=50", Feed, 600, None, true),
        s("reddit-anime-titties", "Reddit · r/anime_titties", Social,
          "https://www.reddit.com/r/anime_titties/new/.rss?limit=50", Feed, 900, None, true),

        // Telegram public channel previews: no API, no key. Opt-in because
        // channel quality varies and the page is HTML, not a stable format.
        s("tg-disclosetv", "Telegram · disclosetv", Social,
          "https://t.me/s/disclosetv", Telegram, 180, None, true),
        s("tg-warmonitors", "Telegram · WarMonitors", Social,
          "https://t.me/s/WarMonitors", Telegram, 180, None, true),

        // --- GDELT: global coverage, but a hard one-request-per-5s public limit.
        // Enabled with --gdelt; the fetcher serialises and backs off on 429.
        s("gdelt-conflict", "GDELT · conflict", News,
          "https://api.gdeltproject.org/api/v2/doc/doc?query=(conflict%20OR%20strike%20OR%20offensive)&mode=artlist&format=json&maxrecords=60&timespan=2h&sort=datedesc",
          Gdelt, 900, None, true),
        s("gdelt-disaster", "GDELT · disaster", News,
          "https://api.gdeltproject.org/api/v2/doc/doc?query=(earthquake%20OR%20flood%20OR%20wildfire%20OR%20cyclone)&mode=artlist&format=json&maxrecords=60&timespan=4h&sort=datedesc",
          Gdelt, 1200, None, true),
    ]
}

/// Health of one source, shown in the sources panel.
#[derive(Debug, Clone)]
pub struct SourceStatus {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub last_ok: Option<chrono::DateTime<chrono::Utc>>,
    pub last_error: Option<String>,
    pub fetches: u64,
    pub failures: u64,
    pub items_total: u64,
    pub last_items: usize,
    pub latency_ms: u64,
}

impl SourceStatus {
    pub fn new(def: &SourceDef) -> Self {
        Self {
            id: def.id.to_string(),
            name: def.name.to_string(),
            kind: def.kind,
            last_ok: None,
            last_error: None,
            fetches: 0,
            failures: 0,
            items_total: 0,
            last_items: 0,
            latency_ms: 0,
        }
    }

    pub fn healthy(&self) -> bool {
        self.last_ok.is_some() && self.last_error.is_none()
    }
}
