//! Best-effort language detection, so a headline that isn't in English or
//! French gets flagged at a glance instead of quietly sitting in the
//! timeline looking like a mistranslation or a broken feed. Detection only —
//! this does not translate anything. A real local translator needs either a
//! bundled machine-translation model (hundreds of MB, a much bigger and
//! riskier thing to get right) or an external service (which would send
//! headline text off the machine, the opposite of what "local" means here).
//! A clear language badge plus `o` to open the original in a browser is the
//! lightweight middle ground.

use whatlang::{detect, Lang};

/// Below this length a detection is closer to a coin flip than a fact — a
/// five-word headline in any Latin-script language looks like passable
/// English noise to a trigram-frequency detector.
const MIN_CHARS: usize = 20;

/// whatlang's own "reliable" threshold (0.9 confidence) rejects a lot of
/// genuine short-headline detections; this is deliberately looser, since a
/// wrong badge here is a cosmetic label, not a wrong fact about the world.
const MIN_CONFIDENCE: f64 = 0.5;

/// The detected language's ISO 639-3 code, uppercased to match this app's
/// other three-letter tags (`UKR`, `MEA`, …) — but only when the text is
/// confidently neither English nor French, so callers can treat `Some` as
/// "show a badge" without a second check.
pub fn detect_foreign(text: &str) -> Option<String> {
    if text.chars().count() < MIN_CHARS {
        return None;
    }
    let info = detect(text)?;
    if matches!(info.lang(), Lang::Eng | Lang::Fra) {
        return None;
    }
    if info.confidence() < MIN_CONFIDENCE {
        return None;
    }
    Some(info.lang().code().to_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_russian() {
        let got = detect_foreign("Российские войска нанесли удар по энергетической инфраструктуре Украины");
        assert_eq!(got.as_deref(), Some("RUS"));
    }

    #[test]
    fn flags_german() {
        let got = detect_foreign("Die Bundesregierung kündigt neue Sanktionen gegen mehrere Unternehmen an");
        assert_eq!(got.as_deref(), Some("DEU"));
    }

    #[test]
    fn does_not_flag_english() {
        assert_eq!(detect_foreign("Russian forces launched a strike on the energy grid overnight"), None);
    }

    #[test]
    fn does_not_flag_french() {
        assert_eq!(
            detect_foreign("Les forces russes ont frappé les infrastructures énergétiques ukrainiennes"),
            None
        );
    }

    #[test]
    fn ignores_text_too_short_to_judge() {
        assert_eq!(detect_foreign("Крым"), None);
        assert_eq!(detect_foreign(""), None);
    }
}
