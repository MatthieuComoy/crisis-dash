//! crisis-dash — a terminal dashboard that watches international news in real
//! time, groups what it finds into situations, and shows each one on a world
//! map next to its full timeline.

mod app;
mod classify;
mod cluster;
mod geo;
mod ingest;
mod lang;
mod model;
mod persist;
mod sources;
mod thumb;
mod ui;

use anyhow::Result;
use app::{App, Overlay, Pane};
use chrono::Utc;
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, EventStream, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::StreamExt;
use ingest::{Engine, IngestConfig, Msg};
use ratatui::prelude::*;
use std::io::stdout;
use std::time::Duration;

const TICK: Duration = Duration::from_millis(250);

#[derive(Debug, Default)]
struct Args {
    include_opt_in: bool,
    min_relevance: Option<u32>,
    only: Vec<String>,
    no_restore: bool,
    list_sources: bool,
    probe: Option<u64>,
    explain: Option<String>,
    help: bool,
}

fn parse_args() -> Args {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--all-sources" | "-a" => a.include_opt_in = true,
            "--min-relevance" => {
                a.min_relevance = it.next().and_then(|v| v.parse().ok());
            }
            "--only" => {
                if let Some(v) = it.next() {
                    a.only = v.split(',').map(|s| s.trim().to_string()).collect();
                }
            }
            "--no-restore" => a.no_restore = true,
            "--list-sources" => a.list_sources = true,
            "--explain" => a.explain = it.next(),
            "--probe" => {
                a.probe = Some(it.next().and_then(|v| v.parse().ok()).unwrap_or(25));
            }
            "-h" | "--help" => a.help = true,
            _ => {}
        }
    }
    a
}

const USAGE: &str = "\
crisis-dash — real-time international crisis monitor

USAGE:
    crisis-dash [OPTIONS]

OPTIONS:
    -a, --all-sources      also poll opt-in sources (GDELT, Reddit, Telegram)
        --only ID,ID       poll only these source ids (see --list-sources)
        --min-relevance N  classifier score an article must reach (default 4;
                           0 keeps everything, 8 keeps only strong matches)
        --no-restore       start with an empty timeline instead of the saved one
        --list-sources     print every source and exit
        --probe [SECS]     fetch once, print what each source yielded and how it
                           clustered, then exit (no TUI) — use to check sources
        --explain TEXT     show how a headline is classified, geolocated and
                           graded; use to tune the rules
    -h, --help             this message

All sources are free and keyless. State is kept in
~/.local/share/crisis-dash/state.json and restored on the next run.
";

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args();

    if args.help {
        print!("{USAGE}");
        return Ok(());
    }
    if args.list_sources {
        for d in sources::registry() {
            println!(
                "{:<22} {:<13} every {:>4}s  {}{}",
                d.id,
                d.kind.label(),
                d.interval,
                if d.opt_in { "[opt-in] " } else { "" },
                d.url
            );
        }
        return Ok(());
    }

    let mut cfg = IngestConfig {
        include_opt_in: args.include_opt_in,
        only: args.only,
        ..Default::default()
    };
    if let Some(m) = args.min_relevance {
        cfg.min_relevance = m;
    }

    if let Some(text) = args.explain {
        explain(&text);
        return Ok(());
    }
    if let Some(secs) = args.probe {
        return probe(cfg, secs).await;
    }

    let now = Utc::now();
    let mut app = App::new(now);
    if !args.no_restore {
        match persist::load() {
            Ok(Some(restored)) => {
                let n = restored.len();
                let items: usize = restored.values().map(|s| s.items.len()).sum();
                for (id, story) in restored {
                    // Anchors already exist; merge rather than replace so a
                    // renamed anchor keeps its history.
                    match app.clusterer.stories.get_mut(&id) {
                        Some(existing) => {
                            for item in story.items {
                                existing.absorb(item);
                            }
                            existing.unread = 0;
                        }
                        None => {
                            app.clusterer.stories.insert(id, story);
                        }
                    }
                }
                for s in app.clusterer.stories.values_mut() {
                    s.unread = 0;
                }
                app.log(format!("restored {n} stories / {items} items"));
            }
            Ok(None) => app.log("no saved state; starting fresh"),
            Err(e) => app.log(format!("could not restore state: {e}")),
        }
    }

    let engine = Engine::start(cfg)?;
    app.log(format!(
        "polling {} sources · {} gazetteer entries",
        engine.active.len(),
        geo::entry_count()
    ));

    let result = run(&mut app, engine).await;

    // Always try to save, even after an error, so a crash does not cost the
    // accumulated timeline.
    if let Err(e) = persist::save(&app.clusterer.stories) {
        eprintln!("crisis-dash: could not save state: {e}");
    }
    result
}

async fn run(app: &mut App, mut engine: Engine) -> Result<()> {
    enable_raw_mode()?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let outcome = event_loop(app, &mut engine, &mut terminal).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;
    outcome
}

async fn event_loop<B: Backend>(
    app: &mut App,
    engine: &mut Engine,
    terminal: &mut Terminal<B>,
) -> Result<()> {
    let mut input = EventStream::new();
    let mut tick = tokio::time::interval(TICK);
    let mut maintenance = tokio::time::interval(Duration::from_secs(60));
    let mut autosave = tokio::time::interval(Duration::from_secs(120));
    // `interval` fires immediately on first poll; skip that for the slow timers.
    maintenance.tick().await;
    autosave.tick().await;

    // A separate client and channel for on-demand thumbnails: unrelated to
    // source polling, so it stays out of the ingest engine entirely. Only
    // ever fetches whichever single entry is on screen — see
    // `maybe_fetch_thumbnail`.
    let thumb_client = reqwest::Client::builder()
        .user_agent(concat!("crisis-dash/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(10))
        .build()?;
    let (thumb_tx, mut thumb_rx) = tokio::sync::mpsc::channel::<thumb::ThumbMsg>(8);

    let size = terminal.size()?;
    let mut layout = ui::compute_layout(Rect::new(0, 0, size.width, size.height));
    let mut dirty = true;

    loop {
        if dirty {
            let now = Utc::now();
            terminal.draw(|f| {
                layout = ui::compute_layout(f.area());
                ui::draw(f, app, now);
            })?;
            dirty = false;
        }

        tokio::select! {
            // Scraper output.
            Some(msg) = engine.rx.recv() => {
                match msg {
                    Msg::Items(items) => {
                        app.ingest(items);
                        for alert in app.take_pending_alerts() {
                            fire_alert(app, &alert);
                        }
                        dirty = true;
                    }
                    Msg::Status(status) => {
                        app.update_status(*status);
                        dirty = true;
                    }
                }
            }

            // Terminal input.
            Some(Ok(event)) = input.next() => {
                match event {
                    Event::Key(key) => {
                        if key.kind == KeyEventKind::Press || key.kind == KeyEventKind::Repeat {
                            handle_key(app, engine, key);
                            dirty = true;
                        }
                    }
                    Event::Mouse(m) => {
                        handle_mouse(app, &layout, m);
                        dirty = true;
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }

            _ = tick.tick() => {
                // Relative timestamps age even when nothing new arrives.
                dirty = true;
            }

            _ = maintenance.tick() => {
                app.clusterer.maintain(Utc::now());
                dirty = true;
            }

            _ = autosave.tick() => {
                if let Err(e) = persist::save(&app.clusterer.stories) {
                    app.log(format!("autosave failed: {e}"));
                }
            }

            Some(msg) = thumb_rx.recv() => {
                match msg {
                    thumb::ThumbMsg::Ready { url, raster } => {
                        app.thumbnails.insert(url, thumb::ThumbState::Ready(raster));
                    }
                    thumb::ThumbMsg::Failed { url } => {
                        app.thumbnails.insert(url, thumb::ThumbState::Failed);
                    }
                }
                dirty = true;
            }
        }

        // Whatever just happened may have changed which entry is on screen;
        // fetch its picture if it has one and we don't already have it.
        maybe_fetch_thumbnail(app, &thumb_client, &thumb_tx);

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Requests the thumbnail for whichever timeline entry is currently on
/// screen, if it has one and isn't already loading or cached. Deliberately
/// never prefetches anything else: a story can carry hundreds of entries, and
/// fetching every image in it would turn a news dashboard into a bandwidth hog.
fn maybe_fetch_thumbnail(app: &mut App, client: &reqwest::Client, tx: &tokio::sync::mpsc::Sender<thumb::ThumbMsg>) {
    let Some(url) = app.wanted_thumbnail() else { return };
    if !app.begin_thumbnail_fetch(&url) {
        return;
    }
    let client = client.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let msg = match thumb::fetch_and_downsample(client, url.clone()).await {
            Ok(raster) => thumb::ThumbMsg::Ready { url, raster },
            Err(_) => thumb::ThumbMsg::Failed { url },
        };
        let _ = tx.send(msg).await;
    });
}

fn handle_key(app: &mut App, engine: &Engine, key: KeyEvent) {
    // Search takes every printable key while active.
    if app.searching {
        match key.code {
            KeyCode::Esc => {
                app.search.clear();
                app.searching = false;
            }
            KeyCode::Enter => app.searching = false,
            KeyCode::Backspace => {
                app.search.pop();
            }
            KeyCode::Char(c) => app.search.push(c),
            _ => {}
        }
        return;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
        app.should_quit = true;
        return;
    }

    // Overlays consume Esc and their own shortcuts first.
    if app.overlay != Overlay::None {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                app.overlay = Overlay::None;
                return;
            }
            KeyCode::Char('o') | KeyCode::Enter if app.overlay == Overlay::Item => {
                open_current(app);
                return;
            }
            KeyCode::Char('y') if app.overlay == Overlay::Item => {
                copy_current(app);
                return;
            }
            KeyCode::Char('s') if app.overlay == Overlay::Sources => {
                app.overlay = Overlay::None;
                return;
            }
            KeyCode::Char('?') if app.overlay == Overlay::Help => {
                app.overlay = Overlay::None;
                return;
            }
            _ => return,
        }
    }

    match key.code {
        KeyCode::Char('q') => app.should_quit = true,
        // Esc backs out of whatever is narrowing the view. It deliberately does
        // not quit: it is the reflex key for "undo that", and quitting on it
        // throws away a session's accumulated timeline.
        KeyCode::Esc => {
            if !app.search.is_empty() {
                app.search.clear();
            } else if app.filter.is_some()
                || app.kind_filter.is_some()
                || app.severity_filter.is_some()
                || app.zone_filter.is_some()
            {
                app.filter = None;
                app.kind_filter = None;
                app.severity_filter = None;
                app.zone_filter = None;
            } else if !app.follow_top {
                app.follow_top = true;
            }
        }

        KeyCode::Up | KeyCode::Char('k') => match app.focus {
            Pane::Timeline => app.scroll_timeline(-1),
            _ => app.move_selection(-1),
        },
        KeyCode::Down | KeyCode::Char('j') => match app.focus {
            Pane::Timeline => app.scroll_timeline(1),
            _ => app.move_selection(1),
        },
        KeyCode::Char('K') => app.scroll_timeline(-1),
        KeyCode::Char('J') => app.scroll_timeline(1),
        KeyCode::PageUp => match app.focus {
            Pane::Timeline => app.scroll_timeline(-8),
            _ => app.move_selection(-10),
        },
        KeyCode::PageDown => match app.focus {
            Pane::Timeline => app.scroll_timeline(8),
            _ => app.move_selection(10),
        },
        KeyCode::Home => app.move_selection(-100_000),
        KeyCode::End => app.move_selection(100_000),

        KeyCode::Tab | KeyCode::Right => {
            app.focus = match app.focus {
                Pane::Stories => Pane::Map,
                Pane::Map => Pane::Timeline,
                Pane::Timeline => Pane::Stories,
            }
        }
        KeyCode::BackTab | KeyCode::Left => {
            app.focus = match app.focus {
                Pane::Stories => Pane::Timeline,
                Pane::Map => Pane::Stories,
                Pane::Timeline => Pane::Map,
            }
        }

        KeyCode::Enter => {
            if app.focus == Pane::Timeline || app.current_item().is_some() {
                app.overlay = Overlay::Item;
            }
        }
        KeyCode::Char('o') => open_current(app),
        KeyCode::Char('y') => copy_current(app),

        KeyCode::Char('f') => {
            app.follow_top = !app.follow_top;
            app.log(if app.follow_top { "following top story" } else { "selection pinned" });
        }
        KeyCode::Char('z') => {
            app.auto_zoom = !app.auto_zoom;
            app.log(if app.auto_zoom { "auto-zoom on" } else { "auto-zoom off" });
        }
        KeyCode::Char('r') => {
            app.region = (app.region + 1) % geo::REGIONS.len();
            app.auto_zoom = false;
        }
        KeyCode::Char('R') => {
            app.region = (app.region + geo::REGIONS.len() - 1) % geo::REGIONS.len();
            app.auto_zoom = false;
        }
        KeyCode::Char('m') => app.map_style = app.map_style.next(),
        KeyCode::Char('c') => app.cycle_filter(true),
        KeyCode::Char('C') => app.cycle_filter(false),
        KeyCode::Char('x') => app.cycle_severity_filter(true),
        KeyCode::Char('X') => app.cycle_severity_filter(false),
        KeyCode::Char('w') => app.cycle_zone_filter(true),
        KeyCode::Char('W') => app.cycle_zone_filter(false),
        KeyCode::Char('v') => app.cycle_kind_filter(),
        KeyCode::Char('t') => app.sort = app.sort.next(),
        KeyCode::Char('/') => {
            app.searching = true;
            app.search.clear();
        }
        KeyCode::Char('s') => app.overlay = Overlay::Sources,
        KeyCode::Char('?') => app.overlay = Overlay::Help,
        KeyCode::Char(' ') => {
            app.paused = !app.paused;
            app.log(if app.paused { "ingestion paused" } else { "ingestion resumed" });
        }
        KeyCode::Char('a') => {
            app.alerts_enabled = !app.alerts_enabled;
            app.log(if app.alerts_enabled { "alerts on" } else { "alerts off" });
        }
        KeyCode::F(5) | KeyCode::Char('g') => {
            engine.refresh_now();
            app.log("refreshing all sources");
        }
        _ => {}
    }
}

fn handle_mouse(app: &mut App, layout: &ui::Layout, m: MouseEvent) {
    let (col, row) = (m.column, m.row);
    let pane = ui::pane_at(layout, col, row);

    match m.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if app.overlay != Overlay::None {
                app.overlay = Overlay::None;
                return;
            }
            match pane {
                Some(Pane::Stories) => {
                    app.focus = Pane::Stories;
                    if let Some(idx) = ui::stories::row_at(app, layout.stories, row) {
                        let clicked = app.visible(Utc::now()).get(idx).map(|s| s.id.clone());
                        if let Some(id) = clicked {
                            app.select_id(&id);
                        }
                    }
                }
                Some(Pane::Map) => {
                    app.focus = Pane::Map;
                    if app.select_nearest_marker(col, row) {
                        app.auto_zoom = true;
                    }
                }
                Some(Pane::Timeline) => {
                    app.focus = Pane::Timeline;
                    // Click an entry to jump straight to its full detail,
                    // rather than requiring select-then-Enter.
                    if let Some(idx) = ui::timeline::row_at(app, layout.timeline, row) {
                        if idx < app.timeline().len() {
                            app.timeline_cursor = idx;
                            app.overlay = Overlay::Item;
                        }
                    }
                }
                None => {}
            }
        }
        MouseEventKind::ScrollDown => match pane {
            Some(Pane::Timeline) => app.scroll_timeline(1),
            Some(Pane::Stories) => app.move_selection(1),
            Some(Pane::Map) => app.region = (app.region + 1) % geo::REGIONS.len(),
            None => {}
        },
        MouseEventKind::ScrollUp => match pane {
            Some(Pane::Timeline) => app.scroll_timeline(-1),
            Some(Pane::Stories) => app.move_selection(-1),
            Some(Pane::Map) => {
                app.region = (app.region + geo::REGIONS.len() - 1) % geo::REGIONS.len()
            }
            None => {}
        },
        _ => {}
    }
}

/// Hand the URL to the desktop. Detached so a slow browser never blocks the UI.
fn open_current(app: &mut App) {
    let Some(url) = app.current_item().map(|i| i.url.clone()) else {
        return;
    };
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    match std::process::Command::new(opener)
        .arg(&url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(_) => app.log(format!("opened {url}")),
        Err(e) => app.log(format!("could not open browser: {e}")),
    }
}

/// Best-effort clipboard copy via whichever helper the system has.
fn copy_current(app: &mut App) {
    let Some(url) = app.current_item().map(|i| i.url.clone()) else {
        return;
    };
    let candidates: [(&str, &[&str]); 4] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
        ("pbcopy", &[]),
    ];
    for (bin, args) in candidates {
        use std::io::Write;
        let child = std::process::Command::new(bin)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        if let Ok(mut c) = child {
            if let Some(mut stdin) = c.stdin.take() {
                let _ = stdin.write_all(url.as_bytes());
            }
            let _ = c.wait();
            app.log(format!("copied url via {bin}"));
            return;
        }
    }
    app.log("no clipboard helper found (wl-copy, xclip, xsel, pbcopy)");
}

/// Headless diagnostic sweep: run the real pipeline once and report what every
/// source returned, what was kept, and how it clustered.
async fn probe(cfg: IngestConfig, secs: u64) -> Result<()> {
    use std::collections::BTreeMap;

    let mut engine = Engine::start(cfg.clone())?;
    let expected = engine.active.len();
    println!(
        "probing {expected} sources for {secs}s (min-relevance {}, opt-in {})\n",
        cfg.min_relevance, cfg.include_opt_in
    );

    let mut app = App::new(Utc::now());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut reported: BTreeMap<String, String> = BTreeMap::new();

    loop {
        tokio::select! {
            Some(msg) = engine.rx.recv() => match msg {
                Msg::Items(items) => app.ingest(items),
                Msg::Status(st) => {
                    let line = match (&st.last_ok, &st.last_error) {
                        (_, Some(e)) => format!("{:<28} FAIL  {}", st.name, e),
                        (Some(_), None) => format!(
                            "{:<28} ok    {:>3} items  {:>5}ms",
                            st.name, st.last_items, st.latency_ms
                        ),
                        _ => format!("{:<28} ...", st.name),
                    };
                    reported.insert(st.id.clone(), line);
                    app.update_status(*st);
                }
            },
            _ = tokio::time::sleep_until(deadline) => break,
        }
        if reported.len() >= expected {
            // Every source has reported once; give stragglers a moment, then stop.
            tokio::time::sleep(Duration::from_millis(600)).await;
            while let Ok(msg) = engine.rx.try_recv() {
                match msg {
                    Msg::Items(items) => app.ingest(items),
                    Msg::Status(st) => app.update_status(*st),
                }
            }
            break;
        }
    }

    println!("── SOURCES ──");
    for line in reported.values() {
        println!("  {line}");
    }

    let now = Utc::now();
    let (ok, total) = app.healthy_sources();
    println!(
        "\n── PIPELINE ──\n  {} items seen, {} kept, {} stories, sources {ok}/{total}",
        app.items_seen,
        app.items_kept,
        app.clusterer.stories.values().filter(|s| !s.items.is_empty()).count()
    );

    let located = app
        .clusterer
        .stories
        .values()
        .filter(|s| !s.items.is_empty() && s.focus().is_some())
        .count();
    let videos: usize = app.clusterer.stories.values().map(|s| s.video_count()).sum();
    println!("  {located} stories geolocated, {videos} videos aggregated");

    println!("\n── TOP STORIES ──");
    for story in app.visible(now).iter().take(18) {
        let place = story.focus().map(|p| p.name).unwrap_or_else(|| "—".into());
        println!(
            "  [{}] {:<44} {:>3} items {:>2} src  {:<20} {}",
            story.category.tag(),
            truncate_probe(&story.title, 44),
            story.items.len(),
            story.sources().len(),
            truncate_probe(&place, 20),
            story.severity.label()
        );
    }

    println!("\n── SAMPLE TIMELINE (top story) ──");
    if let Some(top) = app.visible(now).first() {
        println!("  {}\n", top.title);
        for item in top.items.iter().take(8) {
            println!(
                "  {:>5}  {} {:<24} {}",
                app::ago(now, item.published),
                item.kind.glyph(),
                truncate_probe(&item.source, 24),
                truncate_probe(&item.title, 78)
            );
        }
    }
    Ok(())
}

fn truncate_probe(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
}

/// Show every decision the pipeline makes about one headline.
fn explain(text: &str) {
    println!("text: {text}\n");
    println!("noise:     {}", classify::is_noise(text));
    let (cat, score) = classify::categorize(text);
    println!("category:  {} (score {score})", cat.label());
    match classify::match_anchor(text) {
        Some(a) => println!("anchor:    {} ({})", a.title, a.id),
        None => println!("anchor:    none"),
    }
    println!("severity:  {}", classify::severity_from_text(text, cat).label());
    match geo::locate(text) {
        Some(p) => println!("place:     {} ({:.2}, {:.2})", p.name, p.point.lat, p.point.lon),
        None => println!("place:     unlocated"),
    }
    let places = geo::find_all(text);
    if places.len() > 1 {
        let names: Vec<String> = places.iter().skip(1).take(5).map(|p| p.name.clone()).collect();
        println!("also:      {}", names.join(", "));
    }
    if let Some(t) = classify::extract_toll(text) {
        println!("toll:      {t}");
    }
    println!("tokens:    {}", classify::tokenize(text).join(" "));
}

// ---------------------------------------------------------------- alerts

/// Raises one qualifying story as a real-world alert: a terminal bell (works
/// everywhere, needs nothing external), a desktop notification, and a sound
/// beyond the bell (many terminals mute it or only flash the screen). Every
/// side effect here is best-effort and silently does nothing if the binary or
/// notification daemon isn't present — exactly like `open_current` and
/// `copy_current` above.
fn fire_alert(app: &mut App, alert: &app::PendingAlert) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(b"\x07");
    let _ = std::io::stdout().flush();

    let summary = format!("crisis-dash · {} · {}", alert.severity.label(), alert.reason.label());
    let body = format!("{}  ({})", alert.title, alert.place);

    if cfg!(target_os = "macos") {
        // No shell involved — `arg` passes this as one literal argv element to
        // osascript, so escaping only needs to satisfy AppleScript's own
        // string syntax, not a shell.
        let script = format!(
            "display notification {} with title {}",
            osa_quote(&body),
            osa_quote(&summary)
        );
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(script)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    } else {
        let _ = std::process::Command::new("notify-send")
            .arg("--app-name=crisis-dash")
            .arg("--urgency=critical")
            .arg("--expire-time=15000")
            .arg(&summary)
            .arg(&body)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    play_alert_sound();

    app.log(format!(
        "ALERT [{}] {} {} — {}",
        alert.reason.label(),
        alert.severity.label(),
        alert.story_id,
        alert.title
    ));
}

/// Best-effort audible cue. Tries the desktop's own configured alert sound
/// first (no path-guessing needed), then falls back to a direct file through
/// whichever media server is running.
fn play_alert_sound() {
    if cfg!(target_os = "macos") {
        // Ships with every macOS install; no theme-detection needed.
        try_spawn("afplay", &["/System/Library/Sounds/Glass.aiff"]);
        return;
    }
    if try_spawn("canberra-gtk-play", &["-i", "dialog-warning"]) {
        return;
    }
    const FILES: &[&str] = &[
        "/usr/share/sounds/freedesktop/stereo/dialog-warning.oga",
        "/usr/share/sounds/ocean/stereo/dialog-warning.oga",
        "/usr/share/sounds/freedesktop/stereo/bell.oga",
    ];
    for file in FILES {
        if !std::path::Path::new(file).exists() {
            continue;
        }
        if try_spawn("paplay", &[file]) || try_spawn("pw-play", &[file]) {
            return;
        }
    }
}

/// Wraps a string as an AppleScript string literal for `osascript -e`.
fn osa_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn try_spawn(bin: &str, args: &[&str]) -> bool {
    std::process::Command::new(bin)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .is_ok()
}
