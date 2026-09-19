//! A single tolerant XML feed parser covering RSS 2.0, RDF, Atom, and the
//! namespaced extensions we care about (`geo:`, `georss:`, `gdacs:`, `media:`,
//! `yt:`). Feeds in the wild are inconsistent enough that matching on local
//! names and collecting everything is more robust than modelling each format.

use anyhow::Result;
use quick_xml::events::Event;
use quick_xml::Reader;

/// One `<item>` or `<entry>`, flattened to `(local_name, value)` pairs.
/// Values from attributes are stored as `name@attr`.
#[derive(Debug, Default, Clone)]
pub struct Record {
    pub fields: Vec<(String, String)>,
}

impl Record {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(k, v)| k == key && !v.trim().is_empty())
            .map(|(_, v)| v.as_str())
    }

    /// First non-empty value among several candidate keys.
    pub fn first(&self, keys: &[&str]) -> Option<&str> {
        keys.iter().find_map(|k| self.get(k))
    }

    fn push(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let value = value.into();
        if !value.trim().is_empty() {
            self.fields.push((key.into(), value));
        }
    }
}

fn local_name(raw: &[u8]) -> String {
    let s = String::from_utf8_lossy(raw);
    match s.rsplit_once(':') {
        Some((_, local)) => local.to_ascii_lowercase(),
        None => s.to_ascii_lowercase(),
    }
}

/// Namespace-qualified name, lowercased (`gdacs:alertlevel`). Needed to tell
/// `geo:long` from a hypothetical other `long`, and to read `yt:videoId`.
fn qualified_name(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).to_ascii_lowercase()
}

const ITEM_TAGS: [&str; 2] = ["item", "entry"];

/// Parse a feed document into records. Unknown elements are kept, so a source
/// with custom fields stays usable without changing this function.
pub fn parse(xml: &str) -> Result<Vec<Record>> {
    let mut reader = Reader::from_str(xml);
    let config = reader.config_mut();
    config.trim_text(true);
    config.check_end_names = false;

    let mut records = Vec::new();
    let mut current: Option<Record> = None;
    // Element path inside the current record, so `<author><name>` becomes `author.name`.
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let local = local_name(e.name().as_ref());
                let qual = qualified_name(e.name().as_ref());
                if current.is_none() && ITEM_TAGS.contains(&local.as_str()) {
                    current = Some(Record::default());
                    path.clear();
                    text.clear();
                    continue;
                }
                if let Some(rec) = current.as_mut() {
                    push_attributes(rec, &local, &qual, &e);
                    path.push(local);
                    text.clear();
                }
            }
            Ok(Event::Empty(e)) => {
                // Self-closing elements carry everything in their attributes:
                // <media:thumbnail url="..."/>, <link href="..."/>.
                let local = local_name(e.name().as_ref());
                let qual = qualified_name(e.name().as_ref());
                if let Some(rec) = current.as_mut() {
                    push_attributes(rec, &local, &qual, &e);
                }
            }
            Ok(Event::Text(e)) => {
                if current.is_some() {
                    if let Ok(t) = e.unescape() {
                        text.push_str(&t);
                    }
                }
            }
            Ok(Event::CData(e)) => {
                if current.is_some() {
                    text.push_str(&String::from_utf8_lossy(e.as_ref()));
                }
            }
            Ok(Event::End(e)) => {
                let local = local_name(e.name().as_ref());
                if current.is_some() && ITEM_TAGS.contains(&local.as_str()) && path.is_empty() {
                    if let Some(rec) = current.take() {
                        if !rec.fields.is_empty() {
                            records.push(rec);
                        }
                    }
                    continue;
                }
                if let Some(rec) = current.as_mut() {
                    if !text.trim().is_empty() {
                        let key = path.join(".");
                        let key = if key.is_empty() { local.clone() } else { key };
                        rec.push(key.clone(), text.trim());
                        // Also index by leaf name so callers can ask for "lat"
                        // without knowing it sat under <geo:Point>.
                        if key != local {
                            rec.push(local.clone(), text.trim());
                        }
                    }
                    text.clear();
                    path.pop();
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break, // Truncated or malformed tail: keep what we parsed.
            _ => {}
        }
    }

    // A feed that closes its last <item> implicitly still yields its record.
    if let Some(rec) = current {
        if !rec.fields.is_empty() {
            records.push(rec);
        }
    }
    Ok(records)
}

fn push_attributes(
    rec: &mut Record,
    local: &str,
    qual: &str,
    e: &quick_xml::events::BytesStart<'_>,
) {
    for attr in e.attributes().flatten() {
        let key = local_name(attr.key.as_ref());
        let value = attr
            .unescape_value()
            .map(|v| v.to_string())
            .unwrap_or_else(|_| String::from_utf8_lossy(attr.value.as_ref()).to_string());
        if value.trim().is_empty() {
            continue;
        }
        rec.push(format!("{local}@{key}"), value.clone());
        if qual != local {
            rec.push(format!("{qual}@{key}"), value);
        }
    }
}

/// Strip HTML tags and collapse whitespace — RSS descriptions are frequently
/// escaped HTML fragments, which look terrible in a terminal.
pub fn clean_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            // A removed tag was a word boundary: `</p><p>` separates sentences,
            // and dropping it silently glues "#Ukraine" to "Russian attack".
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let decoded = decode_entities(&out);
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode the handful of entities that survive a round-trip through CDATA.
fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = match tail.find(';') {
            Some(e) if e <= 10 => e,
            _ => {
                out.push('&');
                rest = &tail[1..];
                continue;
            }
        };
        let entity = &tail[1..end];
        let replacement = match entity {
            "amp" => Some("&".to_string()),
            "lt" => Some("<".to_string()),
            "gt" => Some(">".to_string()),
            "quot" => Some("\"".to_string()),
            "apos" | "#39" => Some("'".to_string()),
            "nbsp" => Some(" ".to_string()),
            "hellip" => Some("…".to_string()),
            "mdash" => Some("—".to_string()),
            "ndash" => Some("–".to_string()),
            "rsquo" | "#8217" => Some("'".to_string()),
            "lsquo" | "#8216" => Some("'".to_string()),
            "ldquo" | "#8220" => Some("\"".to_string()),
            "rdquo" | "#8221" => Some("\"".to_string()),
            e if e.starts_with("#x") || e.starts_with("#X") => u32::from_str_radix(&e[2..], 16)
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string()),
            e if e.starts_with('#') => e[1..]
                .parse::<u32>()
                .ok()
                .and_then(char::from_u32)
                .map(|c| c.to_string()),
            _ => None,
        };
        match replacement {
            Some(r) => out.push_str(&r),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rss_with_geo_namespace() {
        let xml = r#"<rss><channel><item>
            <title>Green flood alert in Thailand</title>
            <link>https://example.org/1</link>
            <pubDate>Fri, 18 Sep 2026 06:16:34 GMT</pubDate>
            <gdacs:alertlevel>Green</gdacs:alertlevel>
            <geo:Point><geo:lat>17.10</geo:lat><geo:long>98.99</geo:long></geo:Point>
        </item></channel></rss>"#;
        let recs = parse(xml).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].get("title").unwrap(), "Green flood alert in Thailand");
        assert_eq!(recs[0].get("alertlevel").unwrap(), "Green");
        assert_eq!(recs[0].get("lat").unwrap(), "17.10");
        assert_eq!(recs[0].get("long").unwrap(), "98.99");
    }

    #[test]
    fn parses_atom_link_attribute_and_youtube_id() {
        let xml = r#"<feed><entry>
            <title>Report from the front</title>
            <link rel="alternate" href="https://youtu.be/abc"/>
            <yt:videoId>abc</yt:videoId>
            <published>2026-09-18T06:16:34+00:00</published>
            <media:description>A description</media:description>
        </entry></feed>"#;
        let recs = parse(xml).unwrap();
        assert_eq!(recs[0].get("link@href").unwrap(), "https://youtu.be/abc");
        assert_eq!(recs[0].get("videoid").unwrap(), "abc");
        assert_eq!(recs[0].get("description").unwrap(), "A description");
    }

    #[test]
    fn strips_html_and_decodes_entities() {
        let got = clean_text("<p>Fire &amp; smoke &#8217;near&#8217; the &lt;border&gt;</p>");
        assert_eq!(got, "Fire & smoke 'near' the <border>");
    }

    #[test]
    fn tag_boundaries_become_word_boundaries() {
        // Without this, hashtag-terminated paragraphs merge into the next word.
        let got = clean_text("<p>Hit near Kyiv #Ukraine</p><p>Russian attack reported</p>");
        assert_eq!(got, "Hit near Kyiv #Ukraine Russian attack reported");
    }

    #[test]
    fn unterminated_entity_is_left_alone() {
        assert_eq!(clean_text("Q&A session"), "Q&A session");
    }
}
