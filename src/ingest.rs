//! The scraping engine: one task per source, each on its own cadence, all
//! funnelling into a single channel the UI drains without ever blocking.

use crate::model::Item;
use crate::sources::{self, parsers, Parser, SourceDef, SourceStatus};
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, Notify};

/// What an ingest task reports back to the application.
#[derive(Debug)]
pub enum Msg {
    Items(Vec<Item>),
    Status(Box<SourceStatus>),
}

#[derive(Debug, Clone)]
pub struct IngestConfig {
    /// Include sources marked `opt_in` (GDELT, Reddit, Telegram).
    pub include_opt_in: bool,
    /// Classifier score an editorial item must reach to be kept.
    pub min_relevance: u32,
    /// Only these source ids, when non-empty.
    pub only: Vec<String>,
    pub user_agent: String,
    pub request_timeout: Duration,
}

impl Default for IngestConfig {
    fn default() -> Self {
        Self {
            include_opt_in: false,
            min_relevance: 4,
            only: Vec::new(),
            user_agent: concat!(
                "crisis-dash/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/; terminal news aggregator)"
            )
            .to_string(),
            request_timeout: Duration::from_secs(20),
        }
    }
}

/// GDELT publishes a hard limit of one request every five seconds per client,
/// and answers violations with prose instead of JSON. Every GDELT task waits on
/// this shared gate so the whole process stays within the limit.
#[derive(Clone)]
struct RateGate {
    last: Arc<Mutex<Option<Instant>>>,
    min_spacing: Duration,
}

impl RateGate {
    fn new(min_spacing: Duration) -> Self {
        Self { last: Arc::new(Mutex::new(None)), min_spacing }
    }

    async fn acquire(&self) {
        let mut guard = self.last.lock().await;
        if let Some(prev) = *guard {
            let elapsed = prev.elapsed();
            if elapsed < self.min_spacing {
                tokio::time::sleep(self.min_spacing - elapsed).await;
            }
        }
        *guard = Some(Instant::now());
    }
}

pub struct Engine {
    pub rx: mpsc::Receiver<Msg>,
    /// Notified to make every source fetch immediately.
    refresh: Arc<Notify>,
    pub active: Vec<SourceDef>,
}

impl Engine {
    /// Spawn one task per enabled source and return the receiving end.
    pub fn start(cfg: IngestConfig) -> Result<Engine> {
        let client = reqwest::Client::builder()
            .user_agent(cfg.user_agent.clone())
            .timeout(cfg.request_timeout)
            .connect_timeout(Duration::from_secs(10))
            .gzip(true)
            // Feeds redirect constantly (http->https, cdn shuffles).
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()?;

        let active: Vec<SourceDef> = sources::registry()
            .into_iter()
            .filter(|d| cfg.include_opt_in || !d.opt_in)
            .filter(|d| cfg.only.is_empty() || cfg.only.iter().any(|o| o == d.id))
            .collect();

        if active.is_empty() {
            return Err(anyhow!("no sources selected"));
        }

        // Generous buffer: a first sweep of 40 sources lands nearly at once.
        let (tx, rx) = mpsc::channel(512);
        let refresh = Arc::new(Notify::new());
        let gdelt_gate = RateGate::new(Duration::from_secs(6));

        for (idx, def) in active.iter().enumerate() {
            let task = SourceTask {
                def: def.clone(),
                client: client.clone(),
                tx: tx.clone(),
                refresh: refresh.clone(),
                gate: gdelt_gate.clone(),
                min_relevance: cfg.min_relevance,
                // Spread the opening burst over ~20s so we neither stall the UI
                // nor look like a scraper hammering every host at once.
                startup_delay: Duration::from_millis(120 * idx as u64),
            };
            tokio::spawn(task.run());
        }

        Ok(Engine { rx, refresh, active })
    }

    /// Ask every source to fetch now, ignoring its schedule.
    pub fn refresh_now(&self) {
        self.refresh.notify_waiters();
    }
}

struct SourceTask {
    def: SourceDef,
    client: reqwest::Client,
    tx: mpsc::Sender<Msg>,
    refresh: Arc<Notify>,
    gate: RateGate,
    min_relevance: u32,
    startup_delay: Duration,
}

impl SourceTask {
    async fn run(self) {
        let mut status = SourceStatus::new(&self.def);
        tokio::time::sleep(self.startup_delay).await;

        loop {
            let started = Instant::now();
            status.fetches += 1;

            match self.fetch_once().await {
                Ok(items) => {
                    status.last_ok = Some(Utc::now());
                    status.last_error = None;
                    status.last_items = items.len();
                    status.items_total += items.len() as u64;
                    status.latency_ms = started.elapsed().as_millis() as u64;
                    if !items.is_empty() && self.tx.send(Msg::Items(items)).await.is_err() {
                        return; // UI gone.
                    }
                }
                Err(e) => {
                    status.failures += 1;
                    status.last_items = 0;
                    status.latency_ms = started.elapsed().as_millis() as u64;
                    status.last_error = Some(short_error(&e));
                }
            }

            if self.tx.send(Msg::Status(Box::new(status.clone()))).await.is_err() {
                return;
            }

            // Back off after consecutive failures so a dead host is retried
            // occasionally rather than every cycle.
            let streak = consecutive_failure_factor(&status);
            let wait = Duration::from_secs(self.def.interval).saturating_mul(streak);

            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = self.refresh.notified() => {}
            }
        }
    }

    async fn fetch_once(&self) -> Result<Vec<Item>> {
        if self.def.parser == Parser::Gdelt {
            self.gate.acquire().await;
        }

        let mut req = self.client.get(self.def.url);
        req = match self.def.parser {
            Parser::UsgsGeoJson | Parser::Eonet | Parser::Gdelt | Parser::Bluesky
            | Parser::Mastodon => req.header("Accept", "application/json"),
            Parser::Telegram => req
                .header("Accept", "text/html,application/xhtml+xml")
                // t.me serves the lightweight preview only to browser-like clients.
                .header(
                    "User-Agent",
                    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Firefox/128.0",
                ),
            Parser::Feed | Parser::Gdacs => {
                req.header("Accept", "application/rss+xml, application/atom+xml, application/xml;q=0.9, */*;q=0.8")
            }
        };

        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!("HTTP {}", status.as_u16()));
        }
        let body = resp.text().await?;
        if body.trim().is_empty() {
            return Err(anyhow!("empty body"));
        }

        parsers::build_items(&self.def, &body, Utc::now(), self.min_relevance)
    }
}

/// Multiplier applied to the poll interval while a source keeps failing.
fn consecutive_failure_factor(status: &SourceStatus) -> u32 {
    if status.last_error.is_none() {
        return 1;
    }
    // `failures` is cumulative, but a source that fails most of the time is
    // exactly the one we want to slow down.
    let ratio = status.failures as f64 / status.fetches.max(1) as f64;
    match ratio {
        r if r > 0.8 => 8,
        r if r > 0.5 => 4,
        _ => 2,
    }
}

/// Reqwest's Display impl is a paragraph; the status panel has one column.
fn short_error(e: &anyhow::Error) -> String {
    let full = e.to_string();
    let first = full.split(':').next().unwrap_or(&full).trim();
    let s = if first.len() < 4 { full.as_str() } else { first };
    let s = s.replace("error sending request for url", "request failed");
    if s.chars().count() > 48 {
        format!("{}…", s.chars().take(48).collect::<String>())
    } else {
        s
    }
}
