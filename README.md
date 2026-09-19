# crisis-dash

A terminal dashboard that watches international news in real time, groups what
it finds into ongoing situations, pins each one to a world map, and shows its
full aggregated timeline — wire copy, machine-generated hazard alerts, social
posts and news video, interleaved in time order.

Every source is free and needs no API key or account.

```
┌ header: active stories · severity · source health · category legend ┐
├──────────────────┬──────────────────────────────────────────────────┤
│ ACTIVE STORIES   │  WORLD MAP — one glyph per story, zone highlight  │
│ ranked by heat   ├──────────────────────────────────────────────────┤
│ click to select  │  TIMELINE — the selected story, newest first      │
└──────────────────┴──────────────────────────────────────────────────┘
```

## Run

```sh
cargo run --release              # the dashboard
cargo run --release -- --help    # options
```

Needs a terminal at least 60×18; 160×45 or larger is comfortable.

## Installing, so `crisis-dash` is just a command

`cargo run` rebuilds and launches from inside the project directory. To get a
plain `crisis-dash` command that works from anywhere, pick one:

```sh
# Symlink the release binary somewhere already on PATH. Rebuilding in place
# (cargo build --release) keeps the command up to date automatically.
mkdir -p ~/.local/bin
ln -sf "$(pwd)/target/release/crisis-dash" ~/.local/bin/crisis-dash

# Or let Cargo install a standalone copy instead (into ~/.cargo/bin — make
# sure that's on PATH, e.g. `source "$HOME/.cargo/env"` in your shell rc if
# you installed Rust with rustup):
cargo install --path .
```

Either way, `crisis-dash` then just runs, from any directory, in any
terminal.

### Installing on another machine (a Mac, say)

There's no installer to hand someone — it's one self-contained binary, so
there are two ways to get it onto another machine:

**Build from source there.** The whole thing is plain cross-platform Rust
(`ratatui`, `crossterm`, `reqwest` with `rustls` rather than needing OpenSSL,
`tokio`) — nothing here needs Linux specifically, and the alert sound/notify
step already branches on `cfg!(target_os = "macos")` to use `osascript` and
`afplay` instead of the Linux tools. On the Mac: install Rust (`brew install
rust`, or [rustup.rs](https://rustup.rs)), copy this directory over (or clone
it from wherever you keep it), then `cargo install --path .` there, same as
above.

**Or hand over a prebuilt binary — no Rust needed on the other machine at
all.** [`.github/workflows/release.yml`](.github/workflows/release.yml) builds
one for Linux and for both Apple Silicon and Intel Macs (the Intel binary is
cross-compiled from an Apple Silicon runner, rather than using GitHub's
dedicated Intel macOS runners — those have been queueing for a very long time
lately) whenever a version tag is pushed, and attaches all three to that tag's
GitHub Release:

```sh
git tag v0.1.2 && git push --tags     # kicks the workflow off
```

The repo is private, so downloading a release asset needs a GitHub login —
either `gh release download` (uses your own `gh auth`), or open the
[Releases page](https://github.com/MatthieuComoy/crisis-dash/releases) in a
browser you're signed into and click the asset. On the Mac:

```sh
gh release download -R MatthieuComoy/crisis-dash -p 'crisis-dash-macos-aarch64.tar.gz'
tar xzf crisis-dash-macos-aarch64.tar.gz
xattr -d com.apple.quarantine crisis-dash   # unsigned binary; see note below
mv crisis-dash ~/.local/bin/                # or anywhere on PATH
```

Use `crisis-dash-macos-x86_64.tar.gz` instead on an Intel Mac. The binary
isn't Apple-notarized, so Gatekeeper will refuse to run it until that
`xattr` line clears the quarantine flag (or: right-click → Open once, and
confirm the dialog).

## What it watches

37 sources by default, across five kinds:

| Kind | Sources |
|---|---|
| **Hazard registries** | GDACS (orange/red alerts, with coordinates and affected-area bounding boxes), USGS earthquakes M4.5+, NASA EONET |
| **Humanitarian** | ReliefWeb disasters, UN News |
| **News wires** | BBC World, Al Jazeera, Guardian, France 24, Deutsche Welle, NPR, CBS, WSJ, SCMP, Times of Israel, El País, Fox |
| **Video** | YouTube channel feeds for BBC, Al Jazeera, DW, France 24, Reuters, Sky, Guardian, AP, CNA, NBC, WION |
| **Social** | Bluesky author feeds (Reuters, AP, Guardian, Al Jazeera, NPR), Mastodon hashtag timelines (#breakingnews, #ukraine, #gaza, #conflict) |

Four more are **opt-in** with `--all-sources`, because they work but are
unreliable from datacentre IP ranges or depend on scraping HTML:

- **GDELT** DOC API — global coverage, but a hard one-request-per-five-seconds
  public limit. The fetcher serialises all GDELT traffic through a shared gate
  and backs off on HTTP 429.
- **Reddit** r/worldnews, r/anime_titties — keyless RSS, aggressively rate-limited.
- **Telegram** public channels (`t.me/s/…`) — no API and no key; reads the same
  server-rendered preview page a browser gets. Fast for conflict OSINT.

`--list-sources` prints them all with their poll intervals.

### On X and Threads

Neither is available. X removed free read access: the API starts at a paid tier,
the syndication endpoint that used to serve public timelines now returns an
empty body, and public Nitter instances are gone. Threads has no public read API
without Meta app review and publishes no feeds. Bluesky, Mastodon and Telegram
are the keyless real-time substitutes, and they are wired up.

## How it works

```
sources/  one async task per source, each on its own interval
   ↓      tolerant XML + JSON parsers (RSS, RDF, Atom, GeoJSON, AT Protocol, HTML)
classify  category rules · anchor routing · severity grading · tokenisation
   ↓
geo       229-entry gazetteer resolves headlines to coordinates
   ↓
cluster   registry event ids · anchors · weighted similarity
   ↓
app/ui    ranked story list · canvas world map · timeline
```

**Anchors** are the long-running situations a crisis desk tracks by name —
Russia-Ukraine, Israel-Gaza, Sudan, the Red Sea, DR Congo, and a dozen more.
Each declares *decisive* terms that route an item on their own ("Bakhmut" means
one war) and *supporting* terms that need corroboration ("Russia" appears in
trade stories too). Without anchors a single conflict fragments into a new story
per headline.

**Registry event ids** are ground truth. Two USGS reports of one quake share an
event id and join one timeline; two different quakes never merge, however
identical their formulaic titles look.

**Everything else** clusters by weighted similarity: shared vocabulary, category
agreement, geographic proximity and time proximity. Physical events additionally
refuse to merge across more than 600 km, because an earthquake happens in one
place and reports of two of them are worded almost the same.

**Ranking** ("heat") is recency-weighted corroboration: how many reports, how
recent, from how many independent sources, scaled by severity, decayed by
silence. A story nobody has mentioned for a day sinks on its own.

Editorial and social items must clear a classifier score (`--min-relevance`,
default 4) and a noise blocklist before entering. Hazard registries and
humanitarian reporting bypass both — they are crisis-relevant by construction.

## Keys

| Key | Action |
|---|---|
| `↑ ↓` / `j k` | move through the story list |
| `Tab` / `← →` | switch pane (stories · map · timeline) |
| `J` `K`, `PgUp` `PgDn` | scroll the timeline |
| `Enter` | open the entry overlay |
| `o` / `y` | open the entry's URL / copy it |
| click a story | select it and centre the map on it |
| click the map | select the nearest story marker |
| click a timeline entry | open its full detail directly |
| scroll wheel | scroll whichever pane is under the pointer |
| `f` | follow the top-ranked story as the ranking changes |
| `z` | toggle auto-zoom to the selected story |
| `r` `R` / `m` | cycle map region / map glyph style |
| `c` `C` | cycle category filter forward / back |
| `x` `X` | cycle minimum severity filter (watch · elevated · severe · critical) |
| `w` `W` | cycle zone filter (Europe · Middle East · Africa · Asia · Americas · Oceania) |
| `v` / `t` | cycle entry-type filter / sort order |
| `/` | search; `Esc` backs out of search, then every filter, then re-follows |
| `s` / `?` | source health panel / help |
| `space` / `g` | pause ingestion / refresh every source now |
| `q`, `Ctrl-C` | quit |

`m` cycles the map between braille (dense, but needs a font that renders it
cleanly — some terminals show it as noise) and plain-character dot, block and
half-block markers. **Half-block is the default** for exactly that reason; try
`m` if the map ever looks illegible. Story pins are always printed characters
regardless of map style — `#` armed conflict, `*` Middle East, `@` earthquake,
`~` natural disaster, and so on, matching the header legend.

The selected story's affected zone is drawn as a real rectangle when the
source gives one (GDACS attaches an actual bounding box to every alert) and
as an approximating circle otherwise — most sources only ever give a place
name, not a shape. Zoomed into anything smaller than the world, a small
locator inset appears in the map's corner: the whole globe with a box over
the current viewport, since "Ukraine / Russia" or a story's own close-up
crop otherwise gives no sense of where in the world it actually is.

Filters combine: category, severity and zone all apply at once, and the story
list's title bar shows whichever are active (e.g. `ACTIVE STORIES · Earthquake
· SEVERE+ · Asia (4)`). The zone filter is a coordinate bounding-box cascade
(`geo::zone_of`), not a lookup table per place — it's right for the landmasses
that matter, not guaranteed at every coastline.

### Pictures and video

The entry the timeline cursor is on — and only that one — gets its picture
fetched and shown, in the timeline itself and in the full-detail overlay. This
is deliberately lazy: a story can hold hundreds of entries, and fetching every
image in it on load would turn a news dashboard into a bandwidth hog. Move the
cursor (or click another entry) and its picture loads in turn, cached by URL
for the session so revisiting one is instant.

There is no video *playback* — this is a terminal, and no graphics protocol
(Kitty, iTerm2, Sixel) is used or required. What shows is the source's picture
for that entry: a video's cover frame (YouTube's own thumbnail, fetched even
when the feed doesn't supply one directly — every public video has one at a
predictable URL), a photo attached to a Bluesky or Mastodon post, or a hazard
map from GDACS. `o` still opens the entry's real URL in your browser, which is
how you'd actually watch a video. Pictures render as Unicode half-block
characters in truecolor, the same technique terminal image viewers like
`chafa` use without a graphics protocol — it works in any terminal that
already does truecolor, which this UI assumes everywhere else too.

## Alerts

Turned on by default (`a` toggles it; `ALERTS OFF` shows in the header when
it's off). Every incoming report is judged on its own — not the story's
overall severity, which only ever ratchets up and would otherwise mean a
story that once had one bad day alerts on every later item forever. A report
raises an alert when it is:

- severity **elevated or worse** with its place in **Europe**, or
- severity **critical or worse**, anywhere in the world.

This fires the same way whether that report starts a brand new story or lands
further down an already-tracked one's timeline — a fresh escalation in an
ongoing war alerts exactly like a new one breaking out.

When one fires: a terminal bell, a desktop notification (`notify-send` on
Linux, `osascript` on macOS), a sound beyond the bell (many terminals mute it
or just flash the screen), and the view switches straight to that story — map
recentred, timeline in focus — unless you're mid-search or have an overlay
open, in which case only the selection switches underneath so it's there the
moment you're free to look.

A story that just alerted won't alert again for 20 minutes, so five outlets
reporting the same strike within a couple of minutes collapse into one bell
rather than five — not a rule about "new" vs "ongoing" stories, just enough
debounce that simultaneous corroboration of one development isn't a siren.
This rule isn't configurable from the command line, since nothing has asked
for a different one yet.

The bell and the switch always work. The notification and the sound are
best-effort external processes — on Linux, `notify-send`, then
`canberra-gtk-play`/`paplay`/`pw-play` for the sound; on macOS, `osascript`
and `afplay`. On a bare tty with no notification daemon or sound server, they
silently do nothing rather than error.

## Diagnostics

```sh
crisis-dash --probe 30           # one fetch sweep, per-source results, no TUI
crisis-dash --explain "Russian drone strike hits Kharkiv overnight"
```

`--probe` reports what every source returned, how much survived filtering, and
how it clustered. `--explain` shows every decision the pipeline makes about one
headline — category, score, anchor, severity, place, tokens — which is how the
rule tables get tuned.

## State

The timeline is saved to `~/.local/share/crisis-dash/state.json` every two
minutes and on exit, then restored at startup, so a restart does not cost the
accumulated context. Written via a temporary file and rename, so an interrupted
save cannot truncate it. `--no-restore` starts empty.

A story with no update in **two months** is dropped — from the list, from the
save file, and from the clusterer's memory, anchors (Ukraine, Gaza, Sudan…)
included. An anchor never gets deleted just for going quiet for a day or a
week; two months of silence is the dashboard deciding the situation is no
longer "ongoing" in any useful sense. It reappears the moment a new report
about it arrives. Ranking defaults to **latest update first**, not the "heat"
score — `t` cycles to heat (recency-weighted corroboration across sources) or
worst-severity-first if you want either instead.

## Tuning

The rule tables are plain text constants, designed to be edited without touching
logic:

- `src/classify.rs` — category rules, anchors, noise blocklist, severity terms
- `src/geo.rs` — gazetteer and map region presets
- `src/sources/mod.rs` — the source registry

## Limitations

- Classification is keyword-based, not semantic. It is tuned to be roughly
  right at a glance, and `--explain` exists because it sometimes isn't.
- The gazetteer covers newsworthy places, not every settlement. A headline
  naming only an obscure town lands on its country, or nowhere.
- Social sources are the fastest signal and the least reliable one. Treat an
  unattributed Mastodon or Telegram post as a lead, not a fact — the source and
  handle are always shown on the entry for that reason.
- Reddit and GDELT frequently refuse datacentre IPs; they are opt-in for that
  reason and show as failed in the source panel rather than silently vanishing.
- The zone filter is an approximate coordinate cascade, not an atlas — a
  handful of transcontinental or borderline places (Turkey, the Caucasus,
  Russia) land in whichever zone their specific gazetteer point falls into,
  which will not always match how a given news desk would categorise them.
- Pictures are only ever fetched for the one entry on screen, so a fast-moving
  timeline shows "loading preview…" briefly rather than an instantly-ready
  image on every entry — that trade-off is deliberate, not a bug.
