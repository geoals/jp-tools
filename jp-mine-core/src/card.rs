//! The fields of a mined card, built the way Yomitan builds them.
//!
//! Shared because a card is a card: read-stats mines from the VN overlay and
//! yt-mine from a transcript, but both write the same note type, and the
//! markup here is the part the note type's CSS descends through. A second
//! implementation of it would be a second thing to keep in step with a
//! template nobody edits.
//!
//! What is *not* here is how a card reaches Anki. read-stats routes every add
//! through its own `services::card::add_note`, which fires vn-capture and the
//! completion notification; yt-mine attaches a screenshot and an audio clip cut from the video.
//! Those are two different pipelines and stay that way.

use jp_core::dictionary::html::html_escape;
use jp_core::knowledge::dictionaries;
use sqlx::SqlitePool;

/// The dictionaries that go on the card: a title prefix, and the class name the
/// note type's CSS lists for it.
///
/// The note type styles `.dict-<class>-title` and `.dict-<class>-body` for these
/// two and no others, so a third would land on the card as an unstyled block.
/// The popup is where a dictionary goes to be read; this field is what the card
/// keeps, and the two lists are allowed to differ.
///
/// The class name is not derived from the title. Yomitan's own Jitendex is
/// titled `Jitendex.org [2026-02-05]`, so a slug built from a title changes on
/// every release and stops matching, and the rules that hide Jitendex's star
/// and its ① ② numbering are written against `.dict-jitendex-body` alone. The
/// match is on prefix for the same reason: the version moves, the dictionary
/// does not.
const CARD_DICTIONARIES: [(&str, &str); 2] =
    [("三省堂国語辞典", "sanseido"), ("Jitendex", "jitendex")];

/// The class name the note type styles this dictionary under, if the card
/// carries it at all.
pub fn card_class(title: &str) -> Option<&'static str> {
    CARD_DICTIONARIES
        .iter()
        .find(|(prefix, _)| title.starts_with(prefix))
        .map(|(_, class)| *class)
}

/// One dictionary's block of `VocabDefFull`, in Yomitan's markup.
///
/// The nesting is the load-bearing part: the note type reaches into the
/// glossary through the body div (`.dict-jitendex-body > div > ol > li > i` is
/// what hides Jitendex's star), so a glossary that is the body's sibling rather
/// than its child renders unstyled.
pub fn dict_block(class: &str, title: &str, definitions: &[&str]) -> String {
    let title = html_escape(title);
    let mut out = format!(
        "<div class=\"dict-{class}-title\">{title}</div>\
         <div class=\"dict-{class}-body\">\
         <div style=\"text-align: left;\" class=\"yomitan-glossary\"><ol>"
    );
    for def in definitions {
        out.push_str(&format!("<li data-dictionary=\"{title}\">{def}</li>"));
    }
    out.push_str("</ol></div></div>");
    out
}

/// `VocabDefFull`: one block per card dictionary that has the term.
///
/// Sankoku then Jitendex, in install order, which is the order Yomitan writes
/// them in. The wrapper divs are its markup, reproduced exactly, because the
/// note type styles them per dictionary — `.dict-jitendex-body` is what hides
/// Jitendex's star and its ① ② numbering, and it only matches through the
/// `body` div wrapping the glossary.
pub async fn glossary(pool: &SqlitePool, term: &str, reading: &str) -> Result<String, sqlx::Error> {
    let mut glossary = String::new();
    for dict in dictionaries::list_dictionaries(pool).await? {
        let Some(class) = card_class(&dict.title) else {
            continue;
        };
        let entries = dictionaries::lookup_dictionary_entries(pool, dict.id, term).await?;
        // Same rule as the popup: keep the asked-for reading where the
        // dictionary lists it, and where it does not, every reading beats
        // nothing — Sudachi and a dictionary can disagree about how a word is
        // read, and dropping the entry left the card with no definition at all.
        let senses: Vec<_> = if entries.iter().any(|e| e.reading == reading) {
            entries.iter().filter(|e| e.reading == reading).collect()
        } else {
            entries.iter().collect()
        };
        let definitions: Vec<&str> = senses
            .iter()
            .flat_map(|e| e.definitions.iter().map(String::as_str))
            .collect();
        if definitions.is_empty() {
            continue;
        }
        glossary.push_str(&dict_block(class, &dict.title, &definitions));
    }
    Ok(glossary)
}

/// The first pitch accent any installed dictionary lists for this reading.
///
/// One accent, because `markPitch()` reads a single digit and a card claiming
/// two patterns would colour itself by whichever came first anyway.
pub async fn accent(
    pool: &SqlitePool,
    term: &str,
    reading: &str,
) -> Result<Option<u32>, sqlx::Error> {
    for dict in dictionaries::list_dictionaries(pool).await? {
        let entries = dictionaries::lookup_pitch_entries(pool, dict.id, term).await?;
        if let Some(entry) = entries.iter().find(|e| e.reading == reading) {
            return Ok(entry.positions.first().copied());
        }
    }
    Ok(None)
}

/// Anki's furigana syntax: `寸分[すんぶん]`, and a kana word left bare — a
/// reading identical to the spelling would render as an empty ruby.
pub fn furigana(term: &str, reading: &str) -> String {
    if reading.is_empty() || reading == term {
        term.to_string()
    } else {
        format!("{term}[{reading}]")
    }
}

/// The sentence with the surface wrapped in `<b>`, which is what CompactDef
/// reads back out to tag the spelling the reader actually met.
pub fn bold_surface(sentence: &str, surface: &str) -> String {
    match sentence.find(surface) {
        Some(at) if !surface.is_empty() => format!(
            "{}<b>{}</b>{}",
            html_escape(&sentence[..at]),
            html_escape(surface),
            html_escape(&sentence[at + surface.len()..])
        ),
        _ => html_escape(sentence),
    }
}

/// `[2]`, wrapped as Yomitan wraps it. The card's `markPitch()` pulls the first
/// digit out of this with a regex, so the number has to survive as text.
pub fn pitch_num(accent: u32) -> String {
    format!(
        "<span style=\"display:inline;\"><span>[</span><span>{accent}</span><span>]</span></span>"
    )
}

/// The accent drawn over the reading, in Yomitan's markup.
///
/// A mora is high when the pattern says so — heiban rises after the first and
/// stays up, atamadaka is high only on the first, and anything else is high
/// from the second mora until the drop. The mora the pitch falls *after* is the
/// one carrying the right-hand border.
pub fn pitch_pattern(reading: &str, accent: u32) -> String {
    const OVERLINE: &str = "border-color:currentColor;display:block;user-select:none;\
pointer-events:none;position:absolute;top:0.1em;left:0;right:0;height:0;\
border-top-width:0.1em;border-top-style:solid;";
    const DROP: &str = "right:-0.1em;height:0.4em;border-right-width:0.1em;\
border-right-style:solid;";

    let morae = morae(reading);
    let mut out = String::from("<span style=\"display:inline;\">");
    for (i, mora) in morae.iter().enumerate() {
        let at = i as u32 + 1;
        let high = match accent {
            0 => at >= 2,
            1 => at == 1,
            a => (2..=a).contains(&at),
        };
        let falls = accent >= 1 && at == accent;

        let outer = if falls {
            "display:inline-block;position:relative;padding-right:0.1em;margin-right:0.1em;"
        } else {
            "display:inline-block;position:relative;"
        };
        let mark = match (high, falls) {
            (true, true) => format!("{OVERLINE}{DROP}"),
            (true, false) => OVERLINE.to_string(),
            _ => "border-color:currentColor;".to_string(),
        };
        out.push_str(&format!(
            "<span style=\"{outer}\"><span style=\"display:inline;\">{mora}</span>\
<span style=\"{mark}\"></span></span>"
        ));
    }
    out.push_str("</span>");
    out
}

/// Kana split into morae: a small kana rides with the one before it, so きょ is
/// one mora while ん and っ are each their own.
fn morae(reading: &str) -> Vec<String> {
    const SMALL: &str = "ゃゅょぁぃぅぇぉゎャュョァィゥェォヮ";
    let mut out: Vec<String> = Vec::new();
    for c in reading.chars() {
        if SMALL.contains(c)
            && let Some(last) = out.last_mut()
        {
            last.push(c);
        } else {
            out.push(c.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte-for-byte against the `聡い` card Yomitan itself wrote: さ low, と
    /// high and carrying the drop, い low.
    #[test]
    fn pitch_pattern_matches_what_yomitan_wrote() {
        let expected = "<span style=\"display:inline;\"><span style=\"display:inline-block;position:relative;\"><span style=\"display:inline;\">さ</span><span style=\"border-color:currentColor;\"></span></span><span style=\"display:inline-block;position:relative;padding-right:0.1em;margin-right:0.1em;\"><span style=\"display:inline;\">と</span><span style=\"border-color:currentColor;display:block;user-select:none;pointer-events:none;position:absolute;top:0.1em;left:0;right:0;height:0;border-top-width:0.1em;border-top-style:solid;right:-0.1em;height:0.4em;border-right-width:0.1em;border-right-style:solid;\"></span></span><span style=\"display:inline-block;position:relative;\"><span style=\"display:inline;\">い</span><span style=\"border-color:currentColor;\"></span></span></span>";
        assert_eq!(pitch_pattern("さとい", 2), expected);
    }

    /// Heiban: only the first mora is low and nothing drops.
    #[test]
    fn heiban_rises_once_and_never_falls() {
        let out = pitch_pattern("すんぶん", 0);
        assert_eq!(out.matches("border-top-style:solid").count(), 3);
        assert_eq!(out.matches("border-right-style:solid").count(), 0);
    }

    /// Atamadaka: high on the first mora, which is also where it drops.
    #[test]
    fn atamadaka_is_high_on_the_first_mora_only() {
        let out = pitch_pattern("いぬ", 1);
        assert_eq!(out.matches("border-top-style:solid").count(), 1);
        assert_eq!(out.matches("border-right-style:solid").count(), 1);
    }

    #[test]
    fn small_kana_ride_with_the_mora_before_them() {
        assert_eq!(morae("きょう"), vec!["きょ", "う"]);
        assert_eq!(morae("すんぶん"), vec!["す", "ん", "ぶ", "ん"]);
    }

    #[test]
    fn pitch_num_keeps_the_digit_markpitch_looks_for() {
        assert!(pitch_num(0).contains(">0<"));
    }

    /// Both dictionaries are titled with a version the release moves —
    /// Sankoku's edition, Jitendex's date — and the class name must not.
    #[test]
    fn a_versioned_title_still_finds_its_class() {
        assert_eq!(card_class("三省堂国語辞典　第八版"), Some("sanseido"));
        assert_eq!(card_class("Jitendex"), Some("jitendex"));
        assert_eq!(card_class("Jitendex.org [2026-02-05]"), Some("jitendex"));
        assert_eq!(card_class("明鏡国語辞典 第三版"), None);
    }

    /// Byte-for-byte against the wrapper on a card Yomitan itself wrote. The
    /// nesting is what the note type's selectors descend through, so this is
    /// the part that has to match exactly; the definition inside it is the
    /// dictionary's own.
    #[test]
    fn dict_block_wraps_what_yomitan_wraps() {
        assert_eq!(
            dict_block("jitendex", "Jitendex", &["GLOSS"]),
            "<div class=\"dict-jitendex-title\">Jitendex</div>\
             <div class=\"dict-jitendex-body\">\
             <div style=\"text-align: left;\" class=\"yomitan-glossary\">\
             <ol><li data-dictionary=\"Jitendex\">GLOSS</li></ol></div></div>"
        );
    }

    /// The class in the markup is the one the note type styles, never a slug
    /// of the title — the title is only what the block is labelled with.
    #[test]
    fn the_block_is_classed_by_the_note_type_not_the_title() {
        let block = dict_block("jitendex", "Jitendex.org [2026-02-05]", &["GLOSS"]);
        assert!(block.contains("class=\"dict-jitendex-body\""));
        assert!(block.contains(">Jitendex.org [2026-02-05]<"));
    }
}
