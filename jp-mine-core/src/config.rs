use std::env;

/// Anki note type configuration: model name, deck name, and field mapping.
///
/// Each field is `Option<String>` — `Some("FieldName")` means populate that
/// field on the Anki note, `None` means skip it. Defaults are Lapis's field
/// names, since that is the note type a new install is set up with; every one
/// is overridable through `JP_TOOLS_ANKI_FIELD_*` for a note type of your own.
#[derive(Debug, Clone)]
pub struct AnkiConfig {
    pub model_name: String,
    pub deck_name: String,
    pub field_vocab: Option<String>,
    pub field_definition: Option<String>,
    pub field_sentence: Option<String>,
    pub field_image: Option<String>,
    pub field_audio: Option<String>,
    pub field_source: Option<String>,
    pub field_furigana: Option<String>,
    pub field_pitch_num: Option<String>,
    pub field_pitch_pattern: Option<String>,
    pub field_frequency: Option<String>,
    pub field_compact_def: Option<String>,
    pub field_reading: Option<String>,
    /// The reader-facing rank as a plain integer, for sorting a deck by how
    /// common the word is. Separate from `field_frequency`, which is the
    /// rendered pill.
    pub field_freq_sort: Option<String>,
    /// Tags added to every exported note (set per application).
    pub tags: Vec<String>,
}

impl Default for AnkiConfig {
    fn default() -> Self {
        Self {
            model_name: "Lapis".into(),
            deck_name: "Japanese".into(),
            field_vocab: Some("Expression".into()),
            field_definition: Some("Glossary".into()),
            field_sentence: Some("Sentence".into()),
            field_image: Some("Picture".into()),
            field_audio: Some("SentenceAudio".into()),
            field_source: Some("MiscInfo".into()),
            field_furigana: Some("ExpressionFurigana".into()),
            field_pitch_num: Some("PitchPosition".into()),
            field_pitch_pattern: Some("PitchCategories".into()),
            field_frequency: Some("Frequency".into()),
            field_compact_def: Some("MainDefinition".into()),
            field_reading: Some("ExpressionReading".into()),
            field_freq_sort: Some("FreqSort".into()),
            tags: Vec::new(),
        }
    }
}

/// Parse an Anki field mapping env var. Unset = use default, empty = skip field.
fn anki_field(var: &str, default: &str) -> Option<String> {
    match env::var(var) {
        Ok(v) if v.is_empty() => None,
        Ok(v) => Some(v),
        Err(_) => Some(default.into()),
    }
}

impl AnkiConfig {
    /// Load Anki config from `JP_TOOLS_ANKI_*` environment variables, falling
    /// back to the Lapis defaults. `tags` stays empty — set it
    /// per application after loading.
    pub fn from_env() -> Self {
        let defaults = AnkiConfig::default();

        Self {
            model_name: env::var("JP_TOOLS_ANKI_MODEL").unwrap_or(defaults.model_name),
            deck_name: env::var("JP_TOOLS_ANKI_DECK").unwrap_or(defaults.deck_name),
            field_vocab: anki_field(
                "JP_TOOLS_ANKI_FIELD_VOCAB",
                defaults.field_vocab.as_deref().unwrap_or(""),
            ),
            field_definition: anki_field(
                "JP_TOOLS_ANKI_FIELD_DEFINITION",
                defaults.field_definition.as_deref().unwrap_or(""),
            ),
            field_sentence: anki_field(
                "JP_TOOLS_ANKI_FIELD_SENTENCE",
                defaults.field_sentence.as_deref().unwrap_or(""),
            ),
            field_image: anki_field(
                "JP_TOOLS_ANKI_FIELD_IMAGE",
                defaults.field_image.as_deref().unwrap_or(""),
            ),
            field_audio: anki_field(
                "JP_TOOLS_ANKI_FIELD_AUDIO",
                defaults.field_audio.as_deref().unwrap_or(""),
            ),
            field_source: anki_field(
                "JP_TOOLS_ANKI_FIELD_SOURCE",
                defaults.field_source.as_deref().unwrap_or(""),
            ),
            field_furigana: anki_field(
                "JP_TOOLS_ANKI_FIELD_FURIGANA",
                defaults.field_furigana.as_deref().unwrap_or(""),
            ),
            field_pitch_num: anki_field(
                "JP_TOOLS_ANKI_FIELD_PITCH_NUM",
                defaults.field_pitch_num.as_deref().unwrap_or(""),
            ),
            field_pitch_pattern: anki_field(
                "JP_TOOLS_ANKI_FIELD_PITCH_PATTERN",
                defaults.field_pitch_pattern.as_deref().unwrap_or(""),
            ),
            field_frequency: anki_field(
                "JP_TOOLS_ANKI_FIELD_FREQUENCY",
                defaults.field_frequency.as_deref().unwrap_or(""),
            ),
            field_compact_def: anki_field(
                "JP_TOOLS_ANKI_FIELD_COMPACT_DEF",
                defaults.field_compact_def.as_deref().unwrap_or(""),
            ),
            field_reading: anki_field(
                "JP_TOOLS_ANKI_FIELD_READING",
                defaults.field_reading.as_deref().unwrap_or(""),
            ),
            field_freq_sort: anki_field(
                "JP_TOOLS_ANKI_FIELD_FREQ_SORT",
                defaults.field_freq_sort.as_deref().unwrap_or(""),
            ),
            tags: Vec::new(),
        }
    }

    /// Every field the exporter will write, as `(what it holds, its name on the
    /// note type)`. A field left unset is not listed: an empty value means this
    /// note type has no such field, and a check must not report it missing.
    pub fn configured_fields(&self) -> Vec<(&'static str, &str)> {
        [
            ("headword", &self.field_vocab),
            ("definition", &self.field_definition),
            ("gloss", &self.field_compact_def),
            ("sentence", &self.field_sentence),
            ("image", &self.field_image),
            ("audio", &self.field_audio),
            ("source", &self.field_source),
            ("furigana", &self.field_furigana),
            ("reading", &self.field_reading),
            ("pitch position", &self.field_pitch_num),
            ("pitch pattern", &self.field_pitch_pattern),
            ("frequency", &self.field_frequency),
            ("frequency sort", &self.field_freq_sort),
        ]
        .into_iter()
        .filter_map(|(what, name)| name.as_deref().map(|n| (what, n)))
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults are what a fresh install gets, so they are Lapis's names.
    /// An existing note type pins its own through `JP_TOOLS_ANKI_FIELD_*`.
    #[test]
    fn the_defaults_are_the_lapis_note_type() {
        let c = AnkiConfig::default();
        assert_eq!(c.model_name, "Lapis");
        assert_eq!(c.field_vocab.as_deref(), Some("Expression"));
        assert_eq!(c.field_definition.as_deref(), Some("Glossary"));
        assert_eq!(c.field_compact_def.as_deref(), Some("MainDefinition"));
        assert_eq!(c.field_reading.as_deref(), Some("ExpressionReading"));
        assert_eq!(c.field_freq_sort.as_deref(), Some("FreqSort"));
    }
}
