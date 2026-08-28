//! The fields of a mined card, built the way Yomitan builds them.
//!
//! Shared because a card is a card: kotodex-server mines from the VN overlay and
//! yt-mine from a transcript, but both write the same note type, and the
//! markup here is the part the note type's CSS descends through. A second
//! implementation of it would be a second thing to keep in step with a
//! template nobody edits.
//!
//! What is *not* here is how a card reaches Anki. kotodex-server routes every add
//! through its own `services::card::add_note`, which fires vn-capture and the
//! completion notification; yt-mine attaches a screenshot and an audio clip cut from the video.
//! Those are two different pipelines and stay that way.

use jp_core::dictionary::html::html_escape;
use jp_core::knowledge::dictionaries;
use sqlx::SqlitePool;

/// Which markup `VocabDefFull` is written in.
///
/// Two note types want two different things and neither is wrong: Lapis styles
/// Yomitan's `.yomitan-glossary` directly, so a wrapper is noise; the older note
/// type styles per dictionary and reaches *through* the wrapper, so removing it
/// renders the card unstyled. `KOTODEX_ANKI_STYLE` picks one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// Yomitan's glossary block alone, one per dictionary that has the term.
    Lapis,
    /// Per-dictionary title and body wrappers, and only the two dictionaries
    /// the note type has rules for.
    Legacy,
}

impl Style {
    pub fn from_env() -> Style {
        match std::env::var("KOTODEX_ANKI_STYLE").as_deref() {
            Ok("legacy") => Style::Legacy,
            _ => Style::Lapis,
        }
    }
}

/// The dictionaries the legacy note type has rules for: a title prefix, and the
/// class name its CSS lists.
///
/// It styles `.dict-<class>-title` and `.dict-<class>-body` for these two and no
/// others, so a third would land on the card as an unstyled block. The class
/// name is not derived from the title: Yomitan's own Jitendex is titled
/// `Jitendex.org [2026-02-05]`, so a slug built from a title changes on every
/// release and stops matching. The match is on prefix for the same reason — the
/// version moves, the dictionary does not.
const LEGACY_DICTIONARIES: [(&str, &str); 2] =
    [("三省堂国語辞典", "sanseido"), ("Jitendex", "jitendex")];

/// The class name the legacy note type styles this dictionary under, if it
/// carries the dictionary at all.
fn legacy_class(title: &str) -> Option<&'static str> {
    LEGACY_DICTIONARIES
        .iter()
        .find(|(prefix, _)| title.starts_with(prefix))
        .map(|(_, class)| *class)
}

/// Yomitan's glossary block for one dictionary, with no wrapper — what Lapis
/// styles directly.
pub fn glossary_block(title: &str, definitions: &[&str]) -> String {
    let title = html_escape(title);
    let mut out = String::from("<div style=\"text-align: left;\" class=\"yomitan-glossary\"><ol>");
    for def in definitions {
        out.push_str(&format!("<li data-dictionary=\"{title}\">{def}</li>"));
    }
    out.push_str("</ol></div>");
    out
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

/// `VocabDefFull`: one block per dictionary that has the term, in the popup's
/// own order.
///
/// Which dictionaries reach the card is the style's decision. Lapis takes every
/// one that holds a definition — a frequency or pitch dictionary excludes itself
/// by having no term entries, the same rule the popup uses — so a dictionary
/// added later appears without a code change. Legacy takes the two its CSS has
/// rules for and drops the rest, which is what its CSS can render.
pub async fn glossary(pool: &SqlitePool, term: &str, reading: &str) -> Result<String, sqlx::Error> {
    glossary_in(pool, term, reading, Style::from_env()).await
}

pub async fn glossary_in(
    pool: &SqlitePool,
    term: &str,
    reading: &str,
    style: Style,
) -> Result<String, sqlx::Error> {
    let mut glossary = String::new();
    for dict in dictionaries::list_dictionaries(pool).await? {
        let class = legacy_class(&dict.title);
        if style == Style::Legacy && class.is_none() {
            continue;
        }
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
        glossary.push_str(&match style {
            Style::Lapis => glossary_block(&dict.title, &definitions),
            // `class` is Some here: legacy skipped everything else above.
            Style::Legacy => dict_block(class.unwrap_or_default(), &dict.title, &definitions),
        });
    }
    Ok(glossary)
}

/// The first sense a dictionary gives, as plain text.
///
/// What fills the gloss field when there is no API key to write a better one:
/// the note type shows that field as the card's headline, and an empty one
/// reads as a card with no definition even though the full glossary is there.
/// The master dictionary answers when it has the term, since that is the one
/// whose senses are the vocabulary scale.
pub async fn first_sense(
    pool: &SqlitePool,
    term: &str,
    reading: &str,
) -> Result<Option<String>, sqlx::Error> {
    let dicts = dictionaries::list_dictionaries(pool).await?;
    let ordered = dicts
        .iter()
        .filter(|d| d.role == dictionaries::Role::Master)
        .chain(
            dicts
                .iter()
                .filter(|d| d.role != dictionaries::Role::Master),
        );

    for dict in ordered {
        let entries = dictionaries::lookup_dictionary_entries(pool, dict.id, term).await?;
        // The reading narrows the entries only when it matches one, the same
        // rule `glossary_in` uses: a Sudachi reading the dictionary spells
        // differently must not leave the card with nothing.
        let matched = entries.iter().any(|e| e.reading == reading);
        let sense = entries
            .iter()
            .filter(|e| !matched || e.reading == reading)
            .flat_map(|e| e.definitions.iter())
            .map(|d| plain_text(d))
            .find(|d| !d.is_empty());
        if let Some(sense) = sense {
            return Ok(Some(sense));
        }
    }
    Ok(None)
}

/// Yomitan definitions are HTML fragments; the gloss field is one line of text.
fn plain_text(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // Long enough for a real sense, short enough to stay one headline.
    match collapsed.char_indices().nth(200) {
        Some((cut, _)) => format!("{}…", &collapsed[..cut].trim_end()),
        None => collapsed,
    }
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
    #[test]
    fn plain_text_strips_markup_and_collapses_space() {
        assert_eq!(
            super::plain_text("<span class=\"x\">to eat</span>;\n  <i>to drink</i>"),
            "to eat; to drink"
        );
    }

    #[test]
    fn plain_text_cuts_on_a_character_boundary() {
        let long = "日".repeat(300);
        let cut = super::plain_text(&long);
        assert!(cut.ends_with('…'));
        assert_eq!(cut.chars().count(), 201);
    }

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
        assert_eq!(legacy_class("三省堂国語辞典　第八版"), Some("sanseido"));
        assert_eq!(legacy_class("Jitendex"), Some("jitendex"));
        assert_eq!(legacy_class("Jitendex.org [2026-02-05]"), Some("jitendex"));
        assert_eq!(legacy_class("明鏡国語辞典 第三版"), None);
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

    #[test]
    fn lapis_drops_the_wrappers_the_legacy_note_type_reaches_through() {
        let lapis = glossary_block("Jitendex.org [2026-02-05]", &["GLOSS"]);
        assert!(lapis.starts_with("<div style=\"text-align: left;\" class=\"yomitan-glossary\">"));
        assert!(!lapis.contains("dict-"));
        assert!(lapis.contains(">GLOSS<"));

        let legacy = dict_block("jitendex", "Jitendex.org [2026-02-05]", &["GLOSS"]);
        assert!(legacy.contains("class=\"dict-jitendex-body\"><div style="));
    }
}
