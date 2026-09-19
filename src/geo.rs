//! Gazetteer: resolves free-text headlines to coordinates so every story can be
//! placed on the map, and provides the region presets used by the zoom.

use crate::model::{GeoPoint, Place};
use std::sync::LazyLock;

/// `display | alias;alias;... | lat | lon | radius_deg | weight`
///
/// `weight` encodes specificity: cities beat regions beat countries, so
/// "fighting in Kharkiv, Ukraine" pins to Kharkiv rather than the whole country.
const GAZETTEER: &str = "\
# --- Ukraine / Russia theatre ---
Kyiv|kyiv;kiev|50.45|30.52|0.6|3
Kharkiv|kharkiv;kharkov|49.99|36.23|0.7|3
Odesa|odesa;odessa|46.48|30.73|0.6|3
Lviv|lviv|49.84|24.03|0.6|3
Dnipro|dnipro;dnipropetrovsk|48.46|35.04|0.6|3
Zaporizhzhia|zaporizhzhia;zaporizhia;zaporozhye|47.84|35.14|0.8|3
Kherson|kherson|46.64|32.62|0.8|3
Mariupol|mariupol|47.10|37.54|0.5|3
Bakhmut|bakhmut|48.60|38.00|0.4|3
Donetsk|donetsk|48.02|37.80|0.9|3
Luhansk|luhansk;lugansk|48.57|39.31|0.9|3
Pokrovsk|pokrovsk|48.28|37.18|0.4|3
Avdiivka|avdiivka|48.14|37.75|0.3|3
Sumy|sumy|50.91|34.80|0.6|3
Chernihiv|chernihiv|51.50|31.29|0.6|3
Mykolaiv|mykolaiv;nikolaev|46.98|31.99|0.6|3
Kramatorsk|kramatorsk|48.72|37.56|0.4|3
Crimea|crimea;crimean|45.30|34.40|1.6|2
Donbas|donbas;donbass|48.30|38.20|1.8|2
Sevastopol|sevastopol|44.62|33.53|0.4|3
Ukraine|ukraine;ukrainian;ukrainians|49.00|32.00|6.5|1
Moscow|moscow|55.75|37.62|0.8|3
Saint Petersburg|saint petersburg;st petersburg|59.93|30.34|0.7|3
Belgorod|belgorod|50.60|36.59|0.6|3
Kursk|kursk|51.73|36.19|0.8|3
Rostov|rostov|47.24|39.71|0.7|3
Novorossiysk|novorossiysk|44.72|37.77|0.4|3
Vladivostok|vladivostok|43.12|131.89|0.5|3
Russia|russia;russian;russians;kremlin|58.00|60.00|20.0|1
Belarus|belarus;belarusian;minsk|53.71|27.95|3.0|1
Moldova|moldova;chisinau;moldovan|47.01|28.86|1.5|1
Transnistria|transnistria|47.21|29.46|0.8|2
# --- Middle East ---
Gaza|gaza;gaza strip;gazan|31.42|34.36|0.35|3
Rafah|rafah|31.29|34.26|0.2|3
Khan Younis|khan younis;khan yunis|31.34|34.31|0.2|3
Jerusalem|jerusalem|31.78|35.22|0.4|3
Tel Aviv|tel aviv|32.08|34.78|0.4|3
West Bank|west bank;ramallah;jenin;nablus;hebron|31.95|35.30|0.9|2
Israel|israel;israeli;israelis;idf|31.40|34.95|1.4|1
Lebanon|lebanon;lebanese;beirut;hezbollah|33.85|35.60|1.2|1
Southern Lebanon|southern lebanon;south lebanon|33.30|35.40|0.6|2
Syria|syria;syrian;damascus|34.80|38.50|3.2|1
Aleppo|aleppo|36.20|37.16|0.7|3
Idlib|idlib|35.93|36.63|0.6|3
Homs|homs|34.73|36.71|0.6|3
Latakia|latakia;tartus|35.52|35.79|0.5|3
Iran|iran;iranian;tehran;irgc|32.43|53.69|6.0|1
Isfahan|isfahan;esfahan|32.65|51.67|0.5|3
Bandar Abbas|bandar abbas|27.19|56.27|0.4|3
Iraq|iraq;iraqi;baghdad|33.22|43.68|4.0|1
Erbil|erbil;irbil;kurdistan region|36.19|44.01|0.6|3
Mosul|mosul|36.34|43.13|0.5|3
Basra|basra|30.51|47.78|0.5|3
Yemen|yemen;yemeni;houthi;houthis;sanaa|15.55|48.52|3.5|1
Hodeidah|hodeidah;hudaydah|14.80|42.95|0.4|3
Aden|aden|12.78|45.04|0.4|3
Red Sea|red sea;bab al-mandab;bab el-mandeb|18.00|39.50|5.0|2
Strait of Hormuz|strait of hormuz;hormuz|26.57|56.25|1.2|2
Saudi Arabia|saudi arabia;saudi;riyadh;jeddah|23.89|45.08|6.0|1
Qatar|qatar;doha;qatari|25.29|51.53|0.9|1
United Arab Emirates|united arab emirates;uae;dubai;abu dhabi|24.00|54.00|1.8|1
Kuwait|kuwait|29.31|47.48|0.9|1
Bahrain|bahrain;manama|26.07|50.55|0.3|1
Oman|oman;muscat|21.47|55.98|2.8|1
Jordan|jordan;amman;jordanian|31.24|36.51|1.8|1
Turkey|turkey;turkish;turkiye;ankara;erdogan|39.00|35.24|4.5|1
Istanbul|istanbul|41.01|28.98|0.6|3
Egypt|egypt;egyptian;cairo|26.82|30.80|4.5|1
Suez Canal|suez canal;suez|30.50|32.35|0.8|2
Sinai|sinai|29.50|33.80|1.5|2
# --- Africa ---
Sudan|sudan;sudanese;khartoum|15.50|32.53|6.0|1
Darfur|darfur;el fasher;nyala|13.50|24.50|3.0|2
Port Sudan|port sudan|19.62|37.22|0.4|3
South Sudan|south sudan;juba|6.88|31.31|3.5|1
Ethiopia|ethiopia;ethiopian;addis ababa|9.15|40.49|4.5|1
Tigray|tigray;mekelle|13.90|39.10|1.5|2
Amhara|amhara|11.60|38.00|1.5|2
Eritrea|eritrea;asmara|15.18|39.78|1.8|1
Somalia|somalia;somali;mogadishu;al-shabaab;al shabaab|5.15|46.20|4.5|1
Kenya|kenya;kenyan;nairobi|-0.02|37.91|3.0|1
Uganda|uganda;kampala|1.37|32.29|1.8|1
Tanzania|tanzania;dodoma;dar es salaam|-6.37|34.89|4.0|1
Democratic Republic of Congo|democratic republic of congo;dr congo;drc;kinshasa;congolese|-4.04|21.76|7.0|1
North Kivu|north kivu;goma;kivu|-1.68|29.22|1.2|2
M23 zone|m23|-1.50|29.10|1.2|2
Rwanda|rwanda;kigali;rwandan|-1.94|29.87|0.9|1
Burundi|burundi;bujumbura|-3.37|29.92|0.9|1
Nigeria|nigeria;nigerian;abuja;lagos;boko haram|9.08|8.68|4.5|1
Niger|niger;niamey;nigerien|17.61|8.08|5.0|1
Mali|mali;bamako;malian|17.57|-4.00|5.0|1
Burkina Faso|burkina faso;ouagadougou;burkinabe|12.24|-1.56|2.5|1
Chad|chad;ndjamena;chadian|15.45|18.73|5.5|1
Sahel|sahel|15.00|5.00|9.0|2
Libya|libya;libyan;tripoli;benghazi|26.34|17.23|6.0|1
Tunisia|tunisia;tunis;tunisian|33.89|9.54|2.5|1
Algeria|algeria;algiers;algerian|28.03|1.66|7.0|1
Morocco|morocco;rabat;moroccan;casablanca|31.79|-7.09|3.5|1
Western Sahara|western sahara;polisario|24.22|-12.89|3.5|2
Senegal|senegal;dakar|14.50|-14.45|2.0|1
Ghana|ghana;accra|7.95|-1.02|2.0|1
Ivory Coast|ivory coast;cote d'ivoire;abidjan|7.54|-5.55|2.0|1
Guinea|guinea;conakry|9.95|-9.70|2.0|1
Cameroon|cameroon;yaounde;douala|7.37|12.35|3.0|1
Central African Republic|central african republic;bangui|6.61|20.94|3.5|1
Mozambique|mozambique;maputo;cabo delgado|-18.67|35.53|5.0|1
Zimbabwe|zimbabwe;harare|-19.02|29.15|3.0|1
Zambia|zambia;lusaka|-13.13|27.85|3.5|1
Malawi|malawi;lilongwe|-13.25|34.30|2.0|1
Madagascar|madagascar;antananarivo|-18.77|46.87|4.0|1
South Africa|south africa;johannesburg;pretoria;cape town|-28.00|24.00|5.0|1
Angola|angola;luanda|-11.20|17.87|5.0|1
Ghana Gulf|gulf of guinea|3.00|3.00|5.0|2
# --- Asia ---
China|china;chinese;beijing;xi jinping|35.86|104.20|14.0|1
Shanghai|shanghai|31.23|121.47|0.6|3
Hong Kong|hong kong|22.32|114.17|0.3|3
Xinjiang|xinjiang;uyghur;uighur|41.00|85.00|5.0|2
Tibet|tibet;lhasa;tibetan|31.00|89.00|5.0|2
Taiwan|taiwan;taipei;taiwanese|23.70|121.00|1.6|1
Taiwan Strait|taiwan strait|24.50|119.50|2.0|2
Japan|japan;japanese;tokyo|36.20|138.25|5.0|1
South Korea|south korea;seoul;korean peninsula|35.91|127.77|1.8|1
North Korea|north korea;pyongyang;dprk;kim jong un|40.34|127.51|1.8|1
India|india;indian;new delhi;delhi|20.59|78.96|9.0|1
Kashmir|kashmir;srinagar;jammu|34.08|74.80|1.5|2
Pakistan|pakistan;pakistani;islamabad;karachi|30.38|69.35|5.5|1
Balochistan|balochistan;baluchistan;quetta|28.50|65.50|3.5|2
Afghanistan|afghanistan;afghan;kabul;taliban|33.94|67.71|4.5|1
Bangladesh|bangladesh;dhaka|23.68|90.36|2.0|1
Myanmar|myanmar;burma;burmese;naypyidaw;yangon|21.91|95.96|4.5|1
Rakhine|rakhine;rohingya|20.15|93.50|2.0|2
Thailand|thailand;bangkok;thai|15.87|100.99|3.5|1
Cambodia|cambodia;phnom penh|12.57|104.99|2.0|1
Vietnam|vietnam;hanoi;ho chi minh city|14.06|108.28|4.0|1
Laos|laos;vientiane|19.86|102.50|2.5|1
Philippines|philippines;manila;filipino|12.88|121.77|4.5|1
South China Sea|south china sea;spratly;scarborough shoal;paracel|13.00|114.00|6.0|2
Indonesia|indonesia;jakarta;indonesian|-0.79|113.92|12.0|1
Malaysia|malaysia;kuala lumpur|4.21|101.98|3.0|1
Singapore|singapore|1.35|103.82|0.2|3
Sri Lanka|sri lanka;colombo|7.87|80.77|1.5|1
Nepal|nepal;kathmandu|28.39|84.12|1.8|1
Kazakhstan|kazakhstan;astana;almaty|48.02|66.92|8.0|1
Uzbekistan|uzbekistan;tashkent|41.38|64.59|4.0|1
Kyrgyzstan|kyrgyzstan;bishkek|41.20|74.77|2.5|1
Tajikistan|tajikistan;dushanbe|38.86|71.28|2.5|1
Turkmenistan|turkmenistan;ashgabat|38.97|59.56|4.0|1
Armenia|armenia;yerevan;armenian|40.07|45.04|1.2|1
Azerbaijan|azerbaijan;baku;azerbaijani|40.14|47.58|2.0|1
Nagorno-Karabakh|nagorno-karabakh;karabakh;artsakh|39.85|46.75|0.8|2
Georgia|georgia;tbilisi;georgian|42.32|43.36|1.8|1
Mongolia|mongolia;ulaanbaatar|46.86|103.85|8.0|1
# --- Europe ---
United Kingdom|united kingdom;britain;british;london;uk government|54.00|-2.50|4.5|1
France|france;french;paris;macron|46.60|2.30|4.5|1
Germany|germany;german;berlin;bundestag|51.17|10.45|4.0|1
Poland|poland;polish;warsaw|51.92|19.15|3.0|1
Italy|italy;italian;rome|41.87|12.57|4.0|1
Spain|spain;spanish;madrid;barcelona|40.46|-3.75|4.5|1
Portugal|portugal;lisbon|39.40|-8.22|2.5|1
Netherlands|netherlands;dutch;amsterdam;the hague|52.13|5.29|1.5|1
Belgium|belgium;brussels|50.50|4.47|1.2|1
Sweden|sweden;stockholm;swedish|60.13|18.64|5.0|1
Norway|norway;oslo;norwegian|60.47|8.47|5.0|1
Finland|finland;helsinki;finnish|61.92|25.75|4.0|1
Denmark|denmark;copenhagen;danish|56.26|9.50|1.8|1
Baltic states|baltic;estonia;latvia;lithuania;tallinn;riga;vilnius|56.80|24.50|3.5|2
Kaliningrad|kaliningrad|54.71|20.45|0.6|3
Romania|romania;bucharest;romanian|45.94|24.97|2.5|1
Bulgaria|bulgaria;sofia|42.73|25.49|2.0|1
Hungary|hungary;budapest;orban|47.16|19.50|1.8|1
Slovakia|slovakia;bratislava|48.67|19.70|1.5|1
Czech Republic|czech republic;czechia;prague|49.82|15.47|1.5|1
Austria|austria;vienna|47.52|14.55|1.5|1
Switzerland|switzerland;geneva;zurich;bern|46.82|8.23|1.2|1
Greece|greece;athens;greek|39.07|21.82|2.5|1
Serbia|serbia;belgrade;serbian|44.02|21.01|1.5|1
Kosovo|kosovo;pristina|42.60|20.90|0.8|2
Bosnia|bosnia;sarajevo;republika srpska|43.92|17.68|1.5|1
Croatia|croatia;zagreb|45.10|15.20|1.5|1
Ireland|ireland;dublin|53.14|-7.69|1.5|1
Iceland|iceland;reykjavik;grindavik|64.96|-19.02|2.0|1
Cyprus|cyprus;nicosia|35.13|33.43|0.6|1
Mediterranean|mediterranean|36.00|16.00|8.0|2
Black Sea|black sea|43.40|34.30|4.0|2
Arctic|arctic;svalbard|78.00|20.00|12.0|2
# --- Americas ---
United States|united states;washington;white house;pentagon;u.s.;us military|39.83|-98.58|14.0|1
New York|new york;manhattan|40.71|-74.01|0.6|3
Los Angeles|los angeles|34.05|-118.24|0.6|3
Texas|texas;houston|31.00|-99.00|3.5|2
Florida|florida;miami|27.99|-81.76|2.5|2
California|california;san francisco|36.78|-119.42|3.5|2
Alaska|alaska;anchorage|64.20|-149.49|8.0|2
Hawaii|hawaii;honolulu|20.80|-156.33|2.0|2
Canada|canada;ottawa;toronto;canadian|56.13|-106.35|13.0|1
Mexico|mexico;mexican;mexico city|23.63|-102.55|6.5|1
Guatemala|guatemala|15.78|-90.23|1.5|1
Honduras|honduras;tegucigalpa|15.20|-86.24|1.5|1
El Salvador|el salvador;san salvador|13.79|-88.90|0.8|1
Nicaragua|nicaragua;managua|12.87|-85.21|1.5|1
Costa Rica|costa rica;san jose|9.75|-83.75|1.2|1
Panama|panama;panama canal|8.54|-80.78|1.5|1
Cuba|cuba;havana|21.52|-77.78|2.0|1
Haiti|haiti;port-au-prince;haitian|18.97|-72.29|1.0|1
Dominican Republic|dominican republic;santo domingo|18.74|-70.16|1.0|1
Jamaica|jamaica;kingston|18.11|-77.30|0.6|1
Puerto Rico|puerto rico;san juan|18.22|-66.59|0.6|1
Venezuela|venezuela;caracas;venezuelan;maduro|6.42|-66.59|5.0|1
Colombia|colombia;bogota;colombian|4.57|-74.30|4.0|1
Ecuador|ecuador;quito;guayaquil|-1.83|-78.18|2.0|1
Peru|peru;lima;peruvian|-9.19|-75.02|5.0|1
Bolivia|bolivia;la paz|-16.29|-63.59|4.5|1
Chile|chile;santiago;chilean|-35.68|-71.54|9.0|1
Argentina|argentina;buenos aires;argentine|-38.42|-63.62|9.0|1
Brazil|brazil;brasilia;sao paulo;rio de janeiro;brazilian|-14.24|-51.93|12.0|1
Amazon|amazon rainforest;amazonia|-3.50|-62.00|8.0|2
Uruguay|uruguay;montevideo|-32.52|-55.77|2.0|1
Paraguay|paraguay;asuncion|-23.44|-58.44|3.0|1
Guyana|guyana;georgetown;essequibo|4.86|-58.93|2.5|1
Suriname|suriname;paramaribo|3.92|-56.03|2.0|1
# --- Oceania & poles ---
Australia|australia;australian;canberra;sydney;melbourne|-25.27|133.78|13.0|1
New Zealand|new zealand;wellington;auckland|-40.90|174.89|5.0|1
Papua New Guinea|papua new guinea;port moresby|-6.31|143.96|4.0|1
Fiji|fiji;suva|-17.71|178.07|1.5|1
Solomon Islands|solomon islands;honiara|-9.65|160.16|2.0|1
Vanuatu|vanuatu;port vila|-15.38|166.96|2.0|1
Tonga|tonga;nukualofa|-21.18|-175.20|1.2|1
Antarctica|antarctica;antarctic|-82.00|20.00|20.0|2
Pacific Ocean|pacific ocean|0.00|-160.00|30.0|2
Atlantic Ocean|atlantic ocean|10.00|-35.00|20.0|2
Indian Ocean|indian ocean|-20.00|75.00|20.0|2
";

#[derive(Debug, Clone)]
pub struct Entry {
    pub display: String,
    pub aliases: Vec<String>,
    pub point: GeoPoint,
    pub radius_deg: f64,
    pub weight: u8,
}

static ENTRIES: LazyLock<Vec<Entry>> = LazyLock::new(|| {
    let mut out = Vec::new();
    for line in GAZETTEER.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('|').collect();
        if f.len() != 6 {
            continue;
        }
        let (lat, lon, radius, weight) = match (
            f[2].parse::<f64>(),
            f[3].parse::<f64>(),
            f[4].parse::<f64>(),
            f[5].parse::<u8>(),
        ) {
            (Ok(a), Ok(b), Ok(c), Ok(d)) => (a, b, c, d),
            _ => continue,
        };
        let mut aliases: Vec<String> = f[1]
            .split(';')
            .map(|a| a.trim().to_lowercase())
            .filter(|a| !a.is_empty())
            .collect();
        // Longest alias first so "papua new guinea" wins over "guinea".
        aliases.sort_by_key(|a| std::cmp::Reverse(a.len()));
        out.push(Entry {
            display: f[0].to_string(),
            aliases,
            point: GeoPoint::new(lat, lon),
            radius_deg: radius,
            weight,
        });
    }
    out
});

pub fn entry_count() -> usize {
    ENTRIES.len()
}

/// True when `hay[at..at+len]` is not glued to surrounding letters or digits.
fn is_word_match(hay: &[u8], at: usize, len: usize) -> bool {
    let before_ok = at == 0 || !hay[at - 1].is_ascii_alphanumeric();
    let end = at + len;
    let after_ok = end >= hay.len() || !hay[end].is_ascii_alphanumeric();
    before_ok && after_ok
}

fn find_word(hay_lower: &str, needle: &str) -> Option<usize> {
    let bytes = hay_lower.as_bytes();
    let mut from = 0usize;
    while let Some(rel) = hay_lower[from..].find(needle) {
        let at = from + rel;
        if is_word_match(bytes, at, needle.len()) {
            return Some(at);
        }
        from = at + 1;
        if from >= hay_lower.len() {
            break;
        }
    }
    None
}

/// All gazetteer entries mentioned in the text, ranked best-first.
///
/// Ranking is by specificity, then by how early the place appears — a headline
/// names its subject before its context.
pub fn find_all(text: &str) -> Vec<Place> {
    let lower = text.to_lowercase();
    let mut hits: Vec<(u8, usize, usize, &Entry)> = Vec::new();
    for e in ENTRIES.iter() {
        let mut best: Option<(usize, usize)> = None;
        for alias in &e.aliases {
            if let Some(at) = find_word(&lower, alias) {
                let cand = (at, alias.len());
                if best.map_or(true, |(bat, _)| at < bat) {
                    best = Some(cand);
                }
            }
        }
        if let Some((at, alen)) = best {
            hits.push((e.weight, at, alen, e));
        }
    }
    // Higher weight first, then earlier mention, then longer alias.
    hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.cmp(&b.1))
            .then(b.2.cmp(&a.2))
    });
    hits.into_iter()
        .map(|(_, _, _, e)| Place {
            name: e.display.clone(),
            point: e.point,
            radius_deg: e.radius_deg,
        })
        .collect()
}

/// The single most likely location for a headline.
pub fn locate(text: &str) -> Option<Place> {
    find_all(text).into_iter().next()
}

/// Resolve a place by name when a source already told us the country
/// (GDACS, ReliefWeb and EONET all do).
pub fn lookup(name: &str) -> Option<Place> {
    let lower = name.trim().to_lowercase();
    ENTRIES
        .iter()
        .find(|e| e.display.to_lowercase() == lower || e.aliases.iter().any(|a| *a == lower))
        .map(|e| Place {
            name: e.display.clone(),
            point: e.point,
            radius_deg: e.radius_deg,
        })
}

/// Reverse lookup: the nearest known place to a coordinate pair, used to label
/// alerts that arrive with numbers but no usable region name.
pub fn nearest(point: GeoPoint) -> Option<Place> {
    ENTRIES
        .iter()
        .min_by(|a, b| {
            a.point
                .distance_km(&point)
                .partial_cmp(&b.point.distance_km(&point))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|e| Place {
            name: e.display.clone(),
            point,
            radius_deg: e.radius_deg.min(2.0),
        })
}

/// Coarse continent-like bucket for the "filter by zone" feature. Independent
/// from `REGIONS` above (those are camera framings for the map, not a
/// classification) and from the gazetteer (which only gives named places
/// their own point). A bounding-box cascade over raw coordinates instead,
/// since a filter only needs to be roughly right, and the alternative — a
/// region tag on all 229 gazetteer entries — is a lot of upkeep for the same
/// result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Zone {
    Europe,
    MiddleEast,
    Africa,
    Asia,
    Americas,
    Oceania,
    /// Poles, open ocean, anything the boxes below don't claim.
    Other,
}

impl Zone {
    pub const ALL: [Zone; 7] = [
        Zone::Europe,
        Zone::MiddleEast,
        Zone::Africa,
        Zone::Asia,
        Zone::Americas,
        Zone::Oceania,
        Zone::Other,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Zone::Europe => "Europe",
            Zone::MiddleEast => "Middle East",
            Zone::Africa => "Africa",
            Zone::Asia => "Asia",
            Zone::Americas => "Americas",
            Zone::Oceania => "Oceania",
            Zone::Other => "Other",
        }
    }
}

/// Classify a coordinate into a zone. Tested in this order to resolve the
/// overlaps real geography has (the Middle East and the Caucasus sit inside
/// what would otherwise be Europe's or Asia's box too): Middle East and
/// Europe are carved out first since they are the narrowest claims, Africa
/// next, then the Americas by longitude alone, then Oceania is separated from
/// East Asia by requiring both a low latitude and a longitude east of the
/// Philippines/Indonesia, and Asia mops up whatever is left east of Europe.
pub fn zone_of(p: GeoPoint) -> Zone {
    let (lat, lon) = (p.lat, p.lon);
    if (12.0..=42.0).contains(&lat) && (34.0..=63.0).contains(&lon) {
        return Zone::MiddleEast;
    }
    if (34.0..=72.0).contains(&lat) && (-25.0..=45.0).contains(&lon) {
        return Zone::Europe;
    }
    if (-36.0..=38.0).contains(&lat) && (-20.0..=52.0).contains(&lon) {
        return Zone::Africa;
    }
    if (-170.0..=-30.0).contains(&lon) {
        return Zone::Americas;
    }
    if (lon >= 129.0 && lat <= 10.0) || lon <= -150.0 {
        return Zone::Oceania;
    }
    if lon > 45.0 && lon <= 180.0 {
        return Zone::Asia;
    }
    Zone::Other
}

/// Named viewports for the map, cycled with the region key.
pub const REGIONS: &[(&str, [f64; 2], [f64; 2])] = &[
    ("World", [-180.0, 180.0], [-60.0, 80.0]),
    ("Europe", [-12.0, 45.0], [34.0, 71.0]),
    ("Ukraine / Russia", [20.0, 60.0], [42.0, 62.0]),
    ("Middle East", [24.0, 64.0], [11.0, 42.0]),
    ("Africa", [-20.0, 52.0], [-36.0, 38.0]),
    ("South Asia", [60.0, 100.0], [5.0, 40.0]),
    ("East Asia", [95.0, 150.0], [15.0, 50.0]),
    ("Americas", [-130.0, -34.0], [-56.0, 60.0]),
    ("Pacific", [120.0, 240.0], [-45.0, 45.0]),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zone_of_places_known_landmarks_correctly() {
        assert_eq!(zone_of(lookup("Ukraine").unwrap().point), Zone::Europe);
        assert_eq!(zone_of(lookup("Gaza").unwrap().point), Zone::MiddleEast);
        assert_eq!(zone_of(lookup("Sudan").unwrap().point), Zone::Africa);
        assert_eq!(zone_of(lookup("China").unwrap().point), Zone::Asia);
        assert_eq!(zone_of(lookup("United States").unwrap().point), Zone::Americas);
        assert_eq!(zone_of(lookup("Australia").unwrap().point), Zone::Oceania);
    }

    #[test]
    fn zone_of_does_not_confuse_southeast_asia_with_oceania() {
        // Indonesia and the Philippines sit west of Papua New Guinea; without
        // the longitude cut the Oceania box would swallow them.
        assert_eq!(zone_of(lookup("Indonesia").unwrap().point), Zone::Asia);
        assert_eq!(zone_of(lookup("Philippines").unwrap().point), Zone::Asia);
    }

    #[test]
    fn zone_of_falls_back_to_other_for_the_poles() {
        assert_eq!(zone_of(GeoPoint::new(-82.0, 20.0)), Zone::Other);
    }
}
