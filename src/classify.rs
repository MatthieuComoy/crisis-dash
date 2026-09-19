//! Turns a headline into a category, a severity and a set of clustering tokens,
//! and routes it to a long-running "anchor" story when it clearly belongs to one.

use crate::model::{Category, Severity};
use std::collections::HashSet;
use std::sync::LazyLock;

/// `category | weight | keyword;keyword;...`
///
/// Every keyword found adds its rule's weight to that category; the heaviest
/// category wins. Weights are tuned so one unambiguous term (a named war) beats
/// several generic ones ("talks", "summit").
const RULES: &str = "\
UkraineWar|9|ukraine war;war in ukraine;russian invasion;invasion of ukraine
UkraineWar|6|zelensky;zelenskyy;kremlin offensive;russian strike;russian drone;russian missile
UkraineWar|5|kharkiv;kherson;zaporizhzhia;bakhmut;donetsk;luhansk;donbas;mariupol;pokrovsk;avdiivka;kramatorsk
UkraineWar|4|ukrainian forces;ukrainian troops;ukrainian drone;kyiv;crimea;black sea fleet
UkraineWar|3|ukraine;ukrainian;russia;russian
MiddleEast|9|gaza war;israel-hamas;israel hamas;war in gaza
MiddleEast|6|gaza;hamas;hezbollah;houthi;houthis;idf;west bank;rafah;khan younis
MiddleEast|5|israel;israeli;palestinian;palestinians;netanyahu;ceasefire in gaza
MiddleEast|4|lebanon;beirut;iran;tehran;irgc;syria;damascus;yemen;red sea;houthi attack
ArmedConflict|8|civil war;armed conflict;offensive;frontline;front line;rebels seize;junta forces
ArmedConflict|7|airstrike;air strike;shelling;artillery;drone strike;missile strike;bombardment
ArmedConflict|6|killed in fighting;clashes;militants;insurgents;paramilitary;rsf;m23;al-shabaab;boko haram;taliban
ArmedConflict|5|troops;soldiers;military operation;mobilisation;mobilization;war crimes
ArmedConflict|4|army;militia;combat;ambush;siege
Terrorism|9|terror attack;terrorist attack;suicide bombing;suicide bomber
Terrorism|7|islamic state;isis;isil;al-qaeda;al qaeda;claimed responsibility;car bomb;bomb attack
Terrorism|5|hostage;kidnapping;gunman;mass shooting;stabbing attack
Unrest|8|coup;coup attempt;military takeover;state of emergency;martial law
Unrest|7|protests;protesters;riots;rioting;uprising;demonstrations;crackdown
Unrest|5|strike action;unrest;clashes with police;tear gas;curfew
Earthquake|10|earthquake;magnitude;aftershock;seismic;epicentre;epicenter;quake
Earthquake|7|tsunami warning;tsunami
NaturalDisaster|9|volcano;volcanic eruption;landslide;mudslide;avalanche;sinkhole
NaturalDisaster|8|flood;flooding;floods;flash flood;deluge;inundated
NaturalDisaster|8|wildfire;wildfires;bushfire;forest fire
NaturalDisaster|7|cyclone;hurricane;typhoon;tropical storm;storm surge;tornado
NaturalDisaster|6|drought;famine;locust
NaturalDisaster|6|disaster;evacuated;evacuation;rescuers;death toll;collapsed building
WeatherClimate|7|heatwave;heat wave;record temperature;cold snap;blizzard;climate change;global warming
WeatherClimate|5|el nino;la nina;glacier;sea level;emissions;cop29;cop30;carbon
Health|9|outbreak;epidemic;pandemic;cholera;ebola;mpox;measles;polio;bird flu;h5n1
Health|6|who declares;public health emergency;quarantine;vaccination campaign;infections
Health|4|hospital;virus;disease;malnutrition
Migration|8|refugees;refugee camp;displaced;asylum seekers;migrant boat;migrants drowned
Migration|5|migration;border crossing;deportation;smugglers
Cyber|9|cyberattack;cyber attack;ransomware;data breach;hacked;hackers
Cyber|5|malware;phishing;ddos;espionage campaign;spyware
Economy|7|inflation;recession;central bank;interest rates;stock market;currency crisis
Economy|6|sanctions;tariffs;trade war;oil prices;opec;default on debt;imf bailout
Economy|4|economy;economic;gdp;unemployment
Politics|7|election;elections;parliament;prime minister;president;summit;peace talks;negotiations
Politics|5|diplomatic;ambassador;united nations;security council;nato;european union;treaty;resolution
Politics|4|government;minister;vote;policy;sanctions lifted
";

/// `anchor_id | Category | title | decisive;... | supporting;... | home place`
///
/// Anchors are the situations the world has been watching for months. Without
/// them a single conflict fragments into a new "story" per headline, which is
/// exactly what a crisis dashboard must not do.
///
/// A *decisive* term routes an item on its own ("Bakhmut" means one war). A
/// *supporting* term is ambiguous alone ("Russia" appears in trade stories too),
/// so two of them are required.
const ANCHORS: &str = "\
ukraine-war|UkraineWar|Russia-Ukraine war|ukraine;ukrainian;ukrainians;zelensky;zelenskyy;kharkiv;kherson;zaporizhzhia;donetsk;luhansk;donbas;donbass;bakhmut;pokrovsk;avdiivka;kramatorsk;mariupol;odesa;odessa;mykolaiv;chernihiv|russia;russian;kremlin;kyiv;kiev;crimea;moscow;drone strike;missile strike;front line;frontline|Ukraine
gaza-war|MiddleEast|Israel-Gaza war|gaza;gazan;hamas;rafah;khan younis;khan yunis;west bank|israel;israeli;palestinian;palestinians;idf;netanyahu;hostage;hostages;ceasefire|Gaza
lebanon-israel|MiddleEast|Israel-Lebanon / Hezbollah front|hezbollah;southern lebanon;south lebanon|lebanon;lebanese;beirut;israeli strike;cross-border|Southern Lebanon
iran-tensions|MiddleEast|Iran confrontation|irgc;iranian nuclear|iran;iranian;tehran;enrichment;nuclear;hormuz;sanctions|Iran
red-sea|MiddleEast|Red Sea shipping crisis|houthi;houthis;bab al-mandab;bab el-mandeb|red sea;shipping;tanker;cargo ship;aden|Red Sea
syria|MiddleEast|Syria|syria;syrian;damascus;aleppo;idlib;latakia|assad;kurdish forces;sdf;insurgents|Syria
sudan-war|ArmedConflict|Sudan civil war|sudan;sudanese;khartoum;darfur;el fasher;omdurman|rsf;rapid support forces;janjaweed;famine|Sudan
sahel|ArmedConflict|Sahel insurgency|sahel;jnim|mali;burkina faso;niger;junta;wagner;africa corps;jihadist|Sahel
drc-m23|ArmedConflict|DR Congo / M23 conflict|m23;north kivu;goma;bukavu|dr congo;drc;congolese;rwanda;rwandan;kinshasa|North Kivu
myanmar|ArmedConflict|Myanmar civil war|myanmar;burma;burmese;rohingya;naypyidaw|junta;rakhine;arakan army;resistance;yangon|Myanmar
nigeria-lake-chad|ArmedConflict|Nigeria / Lake Chad insurgency|boko haram;iswap|nigeria;nigerian;lake chad;borno;maiduguri;kidnapped|Nigeria
haiti|Unrest|Haiti gang crisis|haiti;haitian;port-au-prince|gangs;gang;kenyan police;transitional council|Haiti
taiwan-strait|Politics|Taiwan Strait tensions|taiwan strait;pla drills|taiwan;taipei;china;beijing;incursion;airspace|Taiwan Strait
korea|Politics|Korean peninsula|north korea;dprk;pyongyang;kim jong un|south korea;seoul;missile test;ballistic;launch|North Korea
south-china-sea|Politics|South China Sea disputes|south china sea;spratly;scarborough shoal;second thomas shoal|philippines;china coast guard;vessel;water cannon|South China Sea
venezuela|Politics|Venezuela crisis|venezuela;venezuelan;maduro;caracas|essequibo;guyana;opposition;election|Venezuela
maghreb|Politics|Maghreb tensions|western sahara;polisario|algeria;morocco;sahrawi;tindouf|Western Sahara
";

#[derive(Debug, Clone)]
struct Rule {
    category: Category,
    weight: u32,
    keywords: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Anchor {
    pub id: String,
    pub category: Category,
    pub title: String,
    /// Any one of these routes an item to this anchor.
    pub decisive: Vec<String>,
    /// Two of these are needed, because each is ambiguous alone.
    pub supporting: Vec<String>,
    /// Gazetteer name of where this situation is, so the map pin never drifts.
    pub home: String,
}

fn parse_category(s: &str) -> Option<Category> {
    Some(match s {
        "UkraineWar" => Category::UkraineWar,
        "MiddleEast" => Category::MiddleEast,
        "ArmedConflict" => Category::ArmedConflict,
        "Terrorism" => Category::Terrorism,
        "Unrest" => Category::Unrest,
        "Earthquake" => Category::Earthquake,
        "NaturalDisaster" => Category::NaturalDisaster,
        "WeatherClimate" => Category::WeatherClimate,
        "Health" => Category::Health,
        "Migration" => Category::Migration,
        "Cyber" => Category::Cyber,
        "Economy" => Category::Economy,
        "Politics" => Category::Politics,
        _ => return None,
    })
}

static PARSED_RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    RULES
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let f: Vec<&str> = line.split('|').collect();
            if f.len() != 3 {
                return None;
            }
            Some(Rule {
                category: parse_category(f[0])?,
                weight: f[1].parse().ok()?,
                keywords: f[2].split(';').map(|k| k.trim().to_lowercase()).collect(),
            })
        })
        .collect()
});

pub static ANCHOR_LIST: LazyLock<Vec<Anchor>> = LazyLock::new(|| {
    ANCHORS
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let f: Vec<&str> = line.split('|').collect();
            if f.len() != 6 {
                return None;
            }
            let split = |s: &str| -> Vec<String> {
                s.split(';')
                    .map(|k| k.trim().to_lowercase())
                    .filter(|k| !k.is_empty())
                    .collect()
            };
            Some(Anchor {
                id: f[0].to_string(),
                category: parse_category(f[1])?,
                title: f[2].to_string(),
                decisive: split(f[3]),
                supporting: split(f[4]),
                home: f[5].trim().to_string(),
            })
        })
        .collect()
});

fn contains_word(hay: &str, needle: &str) -> bool {
    let bytes = hay.as_bytes();
    let mut from = 0;
    while let Some(rel) = hay[from..].find(needle) {
        let at = from + rel;
        let before = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
        let end = at + needle.len();
        let after = end >= bytes.len() || !bytes[end].is_ascii_alphanumeric();
        if before && after {
            return true;
        }
        from = at + 1;
        if from >= hay.len() {
            break;
        }
    }
    false
}

/// Subject matter a crisis desk does not track. A match vetoes the item even
/// when a crisis keyword appears in it — match reports are full of "clashes",
/// "battle" and "collapse".
const NOISE: &str = "\
premier league;champions league;la liga;serie a;bundesliga;world cup;nba;nfl;mlb;nhl;\
formula 1;grand prix;wimbledon;olympics;super bowl;transfer window;half-time;full-time;\
goalkeeper;touchdown;home run;test match;six nations;golf;tennis;cricket;rugby;\
box office;grammy;oscar;academy award;celebrity;red carpet;reality tv;netflix series;\
album release;tour dates;royal family;met gala;fashion week;horoscope;recipe;\
gift guide;deal of the day;best laptops;how to watch;streaming guide;video game;\
spoilers;season finale;dating app;weight loss;skincare";

static NOISE_SET: LazyLock<Vec<String>> =
    LazyLock::new(|| NOISE.split(';').map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty()).collect());

/// True when the text is about something the dashboard should ignore.
pub fn is_noise(text: &str) -> bool {
    let lower = text.to_lowercase();
    NOISE_SET.iter().any(|n| contains_word(&lower, n))
}

/// Score every category over the text; returns the winner and its score.
/// A low score means the classifier is guessing, and the caller may decide the
/// item is not crisis-relevant at all.
pub fn categorize(text: &str) -> (Category, u32) {
    let lower = text.to_lowercase();
    let mut scores: [u32; Category::ALL.len()] = [0; Category::ALL.len()];
    for rule in PARSED_RULES.iter() {
        let hits = rule
            .keywords
            .iter()
            .filter(|k| contains_word(&lower, k))
            .count() as u32;
        if hits > 0 {
            let idx = Category::ALL.iter().position(|c| *c == rule.category).unwrap();
            // Diminishing returns: three synonyms of the same idea are not three facts.
            scores[idx] += rule.weight * hits.min(3);
        }
    }
    let (idx, best) = scores
        .iter()
        .enumerate()
        .max_by_key(|(_, s)| **s)
        .map(|(i, s)| (i, *s))
        .unwrap_or((Category::ALL.len() - 1, 0));
    if best == 0 {
        (Category::Other, 0)
    } else {
        (Category::ALL[idx], best)
    }
}

/// The anchor story this text belongs to, if any.
///
/// One decisive term is enough; otherwise two supporting terms must agree. When
/// several anchors qualify — a strike on Lebanon during the Gaza war — the one
/// with the most evidence wins.
pub fn match_anchor(text: &str) -> Option<&'static Anchor> {
    let lower = text.to_lowercase();
    let mut best: Option<(u32, &'static Anchor)> = None;
    for a in ANCHOR_LIST.iter() {
        let decisive = a.decisive.iter().filter(|k| contains_word(&lower, k)).count();
        let supporting = a.supporting.iter().filter(|k| contains_word(&lower, k)).count();
        if decisive == 0 && supporting < 2 {
            continue;
        }
        // Decisive terms count double so a headline naming the place beats one
        // that merely alludes to the region.
        let score = (decisive as u32) * 2 + supporting as u32;
        if best.map_or(true, |(b, _)| score > b) {
            best = Some((score, a));
        }
    }
    best.map(|(_, a)| a)
}

/// Words too common to help distinguish one story from another.
const STOPWORDS: &str = "the a an and or but of in on at to for from by with as is are was were be been \
being has have had do does did will would can could should may might must not no nor so if then than \
that this these those it its they them their there here what which who whom when where why how all any \
both each few more most other some such only own same too very just about after before over under again \
further once during into through above below out off down up more new news says say said report reports \
reported latest live update updates video watch photos photo world international breaking day week year \
years ago first last next amid after two three four five he she his her him you your we our us i";

static STOPSET: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| STOPWORDS.split_whitespace().collect());

/// Significant, normalised tokens used to measure how similar two headlines are.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.len() < 3 {
            continue;
        }
        let w = raw.to_lowercase();
        if STOPSET.contains(w.as_str()) || w.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if seen.insert(w.clone()) {
            out.push(w);
        }
    }
    out
}

/// Severity inferred from language alone, for sources that do not grade their
/// own output. Authoritative sources (GDACS, USGS) override this.
pub fn severity_from_text(text: &str, category: Category) -> Severity {
    let lower = text.to_lowercase();
    const CRITICAL: &[&str] = &[
        "massacre", "genocide", "nuclear strike", "hundreds killed", "thousands killed",
        "mass casualties", "state of war", "declares war", "full-scale invasion",
    ];
    const SEVERE: &[&str] = &[
        "killed", "dead", "death toll", "casualties", "airstrike", "air strike", "bombing",
        "invasion", "offensive", "massive", "evacuate", "evacuation", "state of emergency",
        "disaster", "catastrophic", "devastating", "collapse", "coup",
    ];
    const ELEVATED: &[&str] = &[
        "wounded", "injured", "attack", "strike", "clashes", "warning", "alert", "threat",
        "escalation", "sanctions", "protest", "outbreak", "displaced", "missile", "drone",
    ];
    if CRITICAL.iter().any(|k| lower.contains(k)) {
        return Severity::Critical;
    }
    if SEVERE.iter().any(|k| lower.contains(k)) {
        return Severity::Severe;
    }
    if ELEVATED.iter().any(|k| lower.contains(k)) {
        return Severity::Elevated;
    }
    match category {
        Category::UkraineWar
        | Category::MiddleEast
        | Category::ArmedConflict
        | Category::Terrorism => Severity::Elevated,
        Category::Other | Category::Politics | Category::Economy => Severity::Info,
        _ => Severity::Watch,
    }
}

/// Casualty figures mentioned in the text, surfaced as a fact on the item.
pub fn extract_toll(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        let digits = w.trim_matches(|c: char| !c.is_ascii_digit());
        let n: u32 = match digits.parse() {
            Ok(n) if n > 0 => n,
            _ => continue,
        };
        let tail = words[i + 1..].iter().take(3).copied().collect::<Vec<_>>().join(" ");
        for kind in ["killed", "dead", "died", "wounded", "injured", "missing", "displaced"] {
            if tail.contains(kind) {
                return Some(format!("{n} {kind}"));
            }
        }
    }
    None
}
