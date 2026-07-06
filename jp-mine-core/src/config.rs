use std::env;

/// Anki note type configuration: model name, deck name, and field mapping.
///
/// Each field is `Option<String>` — `Some("FieldName")` means populate that
/// field on the Anki note, `None` means skip it. Defaults match the
/// "Japanese sentences" note type used by Yomitan.
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
    pub field_frequency: Option<String>,
    pub field_llm_definition: Option<String>,
    /// Tags added to every exported note (set per application).
    pub tags: Vec<String>,
}

impl Default for AnkiConfig {
    fn default() -> Self {
        Self {
            model_name: "Japanese sentences".into(),
            deck_name: "Japanese".into(),
            field_vocab: Some("VocabKanji".into()),
            field_definition: Some("VocabDef".into()),
            field_sentence: Some("SentKanji".into()),
            field_image: Some("Image".into()),
            field_audio: Some("SentAudio".into()),
            field_source: Some("Document".into()),
            field_furigana: Some("VocabFurigana".into()),
            field_pitch_num: Some("VocabPitchNum".into()),
            field_frequency: Some("Frequency".into()),
            field_llm_definition: Some("LLMDef".into()),
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
    /// back to the "Japanese sentences" defaults. `tags` stays empty — set it
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
            field_frequency: anki_field(
                "JP_TOOLS_ANKI_FIELD_FREQUENCY",
                defaults.field_frequency.as_deref().unwrap_or(""),
            ),
            field_llm_definition: anki_field(
                "JP_TOOLS_ANKI_FIELD_LLM_DEFINITION",
                defaults.field_llm_definition.as_deref().unwrap_or(""),
            ),
            tags: Vec::new(),
        }
    }
}
