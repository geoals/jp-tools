//! What identity a token gets: the `(headword, reading)` pair the ledger keys
//! on, and whether it counts.
//!
//! Every case here was measured against the live setup first — the pairs in
//! `master_pairs.tsv` are Sankoku's own, so a failure means the tokenizer
//! changed, not that the dictionary did.
//!
//! `JP_TOOLS_SUDACHI_DICT_PATH=$PWD/../system_full.dic \
//!  cargo test -p jp-core --test identity_resolution -- --ignored`

use std::collections::{HashMap, HashSet};
use std::path::Path;

use jp_core::knowledge::dictionaries::PreferredReading;
use jp_core::text::kana::to_hiragana;
use jp_core::tokenize::{MasterWords, SudachiTokenizer, Token, Tokenizer, counts_as_word};

fn master_entries() -> Vec<(String, String)> {
    include_str!("master_pairs.tsv")
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let (term, reading) = l.split_once('\t').expect("term\\treading");
            (term.to_string(), reading.to_string())
        })
        .collect()
}

/// BCCWJ ranks, only for the headwords a test actually has to arbitrate
/// between. うかがう is 伺う and 窺う and nothing in the sentence can say which.
fn ranks() -> HashMap<(String, String), i64> {
    // Keyed on the pair: a rank belongs to a spelling *read a certain way*.
    [
        ("伺う", "うかがう", 1886),
        ("窺う", "うかがう", 2831),
        ("敵", "てき", 1039),
        ("隙", "すき", 4156),
    ]
    .into_iter()
    .map(|(t, r, n)| ((t.to_string(), r.to_string()), n))
    .collect()
}

/// What `dictionaries::preferred_readings` derives for these words from
/// Jitendex and BCCWJ; that derivation has its own tests.
fn preferences() -> HashMap<String, PreferredReading> {
    [
        ("私", "わたし", vec!["わたし", "あたし", "あたくし"]),
        ("何", "なに", vec!["なに", "なん"]),
        // JMdict scores the free-standing noun of each of these 200 and the
        // bound reading 0, which is what the bound-morpheme guard has to refuse.
        ("名", "な", vec!["な"]),
        ("者", "もの", vec!["もの"]),
        ("生", "なま", vec!["なま"]),
    ]
    .into_iter()
    .map(|(term, preferred, acceptable)| {
        (
            term.to_string(),
            PreferredReading {
                preferred: preferred.to_string(),
                acceptable: acceptable.into_iter().map(|s| s.to_string()).collect(),
            },
        )
    })
    .collect()
}

/// The master headwords Sankoku tags as conjugatable (Yomitan field 3). 許す has
/// a `v5`; 許せ, おいた and 汝 are headwords with none.
fn conjugatable() -> HashSet<String> {
    [
        "許す",
        "慣れる",
        "続く",
        "開く",
        "置く",
        "知る",
        "行く",
        "食べる",
        "待つ",
        "押す",
        "言う",
        "笑う",
        "見る",
        "振り返る",
        "信じる",
        "信ずる",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn setup() -> (SudachiTokenizer, MasterWords) {
    let dict_path = std::env::var("JP_TOOLS_SUDACHI_DICT_PATH")
        .expect("JP_TOOLS_SUDACHI_DICT_PATH must be set");
    let entries = master_entries();
    let lexicon: HashSet<String> = entries.iter().map(|(t, _)| t.clone()).collect();
    // A non-empty deck is what puts the tokenizer on the C→B→A path, which is
    // the one production runs.
    let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::from(["x".to_string()]))
        .unwrap()
        .with_lexicon(lexicon.clone())
        .with_master_readings(&entries)
        .with_frequency(ranks())
        .with_preferred_readings(preferences())
        .with_conjugatable(conjugatable());
    (tokenizer, MasterWords::new(lexicon, &entries))
}

/// `(base_form, reading)` per token, readings folded to hiragana — the ledger's
/// key, which is the only thing these tests are about.
fn identities(tokens: &[Token]) -> Vec<(String, String)> {
    tokens
        .iter()
        .map(|t| (t.base_form.clone(), to_hiragana(&t.reading)))
        .collect()
}

fn tokens_of(tk: &SudachiTokenizer, text: &str) -> Vec<Token> {
    tk.tokenize(text).unwrap()
}

fn identity_of(tokens: &[Token], surface: &str) -> (String, String) {
    let t = tokens
        .iter()
        .find(|t| t.surface == surface)
        .unwrap_or_else(|| panic!("no token with surface {surface}: {tokens:?}"));
    (t.base_form.clone(), to_hiragana(&t.reading))
}

fn pair(term: &str, reading: &str) -> (String, String) {
    (term.to_string(), reading.to_string())
}

/// A shred off an out-of-vocabulary path is not a word, and must not be
/// normalised into one: Sudachi has no とん mimetic, so んっと is left over, and
/// んっと normalises to うんと — a real Sankoku headword, counted 685 times.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_token_with_an_impossible_onset_is_never_rescued_into_a_word() {
    let (tk, master) = setup();
    let tokens = tokens_of(&tk, "そのメルルの胸を、とんっと軽く押した。");

    let bases: Vec<&str> = tokens.iter().map(|t| t.base_form.as_str()).collect();
    assert!(!bases.contains(&"うんと"), "{bases:?}");
    for t in &tokens {
        let onset = t.surface.chars().next().unwrap();
        if matches!(onset, 'っ' | 'ん' | 'ッ' | 'ン') {
            assert!(!counts_as_word(t, &master), "{t:?} must not count");
        }
    }
    // The rest of the line is untouched.
    for (surface, want) in [
        ("胸", ("胸", "むね")),
        ("軽く", ("軽い", "かるい")),
        ("押し", ("押す", "おす")),
    ] {
        assert_eq!(identity_of(&tokens, surface), pair(want.0, want.1));
    }
}

/// A headword whose parts are two particles. Sudachi's Mode C splits it and the
/// content-word fence refuses to put it back; the master listing it is the whole
/// reason it is a word.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn an_expression_the_master_lists_is_rejoined_across_function_words() {
    let (tk, master) = setup();
    let tokens = tokens_of(&tk, "それどころか、彼は笑っていた。");

    let joined = tokens
        .iter()
        .find(|t| t.surface == "それどころか")
        .expect("それどころか must rejoin");
    assert_eq!(
        (joined.base_form.clone(), to_hiragana(&joined.reading)),
        pair("それどころか", "それどころか")
    );
    assert!(counts_as_word(joined, &master));
}

/// て + いた: Sankoku lists this いる as its own kana headword, separate from
/// 居る, and Sudachi's dictionary form already spells it that way.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_kana_subsidiary_goes_to_its_own_kana_headword() {
    let (tk, _) = setup();

    assert_eq!(
        identity_of(&tokens_of(&tk, "それどころか、彼は笑っていた。"), "い"),
        pair("いる", "いる")
    );
    assert_eq!(
        identity_of(&tokens_of(&tk, "食べてみる"), "みる"),
        pair("みる", "みる")
    );
    assert_eq!(
        identity_of(&tokens_of(&tk, "行かなければならない"), "なら"),
        pair("なる", "なる")
    );
}

/// The same rule must not touch a subsidiary written in kanji — 見てみる is 見る
/// once and みる once, not twice of either.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_kanji_subsidiary_keeps_its_kanji_headword() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "見てみる");
    assert_eq!(identity_of(&tokens, "見"), pair("見る", "みる"));
    assert_eq!(identity_of(&tokens, "みる"), pair("みる", "みる"));
}

/// Sudachi normalizes to its own orthography, which the master does not always
/// share. Where the normalized spelling names no listed pair, the surface-faithful
/// one does.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_normalized_spelling_the_master_does_not_list_falls_back_to_the_written_one() {
    let (tk, master) = setup();

    // 一寸 is listed, but only as いっすん.
    let tokens = tokens_of(&tk, "ちょっと待って");
    assert_eq!(
        identity_of(&tokens, "ちょっと"),
        pair("ちょっと", "ちょっと")
    );
    assert_eq!(identity_of(&tokens, "待っ"), pair("待つ", "まつ"));

    // 為る likewise: the commonest verb in the language was off the scale.
    let tokens = tokens_of(&tk, "そうすると決めた");
    assert_eq!(identity_of(&tokens, "する"), pair("する", "する"));
    let suru = tokens.iter().find(|t| t.surface == "する").unwrap();
    assert!(counts_as_word(suru, &master));
}

/// A potential form: Sudachi hands back the base verb's spelling with the
/// potential's reading, and (行く, いける) is a pair no dictionary has. Asking
/// what 行く reads as on its own repairs it.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_potential_form_keeps_its_base_verbs_reading() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "私なら行けるはずだ");
    assert_eq!(identity_of(&tokens, "行ける"), pair("行く", "いく"));
    // 筈/はず *is* a Sankoku pair, so normalisation stands here.
    assert_eq!(identity_of(&tokens, "はず"), pair("筈", "はず"));
}

/// Nothing but the reading is left to go on, and it names two headwords. A
/// frequency pick is sometimes wrong — 隙をうかがう is 窺う — but a word read and
/// credited to its commoner spelling beats one credited to nothing.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn an_unwritten_verb_is_arbitrated_by_frequency() {
    let (tk, master) = setup();
    let tokens = tokens_of(&tk, "敵の隙をうかがう");
    assert_eq!(identity_of(&tokens, "うかがう"), pair("伺う", "うかがう"));
    let t = tokens.iter().find(|t| t.surface == "うかがう").unwrap();
    assert!(counts_as_word(t, &master));
    assert_eq!(identity_of(&tokens, "敵"), pair("敵", "てき"));
    assert_eq!(identity_of(&tokens, "隙"), pair("隙", "すき"));
}

/// A kanji head plus a kana suffix, joined on the reading. The fence that stops
/// そう + する → 相する is about all-kana runs, so this passes under it.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_kanji_headed_compound_joins_on_its_reading() {
    let (tk, master) = setup();
    let tokens = tokens_of(&tk, "綺麗ごとを言うな");
    let joined = tokens
        .iter()
        .find(|t| t.surface == "綺麗ごと")
        .unwrap_or_else(|| panic!("綺麗 + ごと must join: {tokens:?}"));
    assert_eq!(
        (joined.base_form.clone(), to_hiragana(&joined.reading)),
        pair("綺麗事", "きれいごと")
    );
    assert!(counts_as_word(joined, &master));
}

/// Everything that already worked. These pass before the rewrite and have to
/// keep passing after it.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn the_existing_behaviour_is_unchanged() {
    let (tk, _) = setup();

    // One verb, one row, however its stem is spelled.
    for text in ["何も知らない", "知っている", "それを知る"] {
        let ids = identities(&tokens_of(&tk, text));
        assert!(
            ids.contains(&pair("知る", "しる")),
            "{text} lost 知る: {ids:?}"
        );
    }
    // Compounds Sudachi's own lexicon lacks still rejoin, on either signal.
    assert!(
        identities(&tokens_of(&tk, "申し訳ないと言った"))
            .contains(&pair("申し訳ない", "もうしわけない"))
    );
    assert!(
        identities(&tokens_of(&tk, "後ろを振り返った")).contains(&pair("振り返る", "ふりかえる"))
    );
    // A name is neither split nor joined into anything.
    let tokens = tokens_of(&tk, "東京で本を読む");
    assert!(tokens.iter().any(|t| t.surface == "東京" && t.proper_noun));
}

/// Sudachi reads a bare 私 as わたくし in every context tried, and (私, わたくし)
/// is a listed pair, so nothing about validating the pair can reach it. The
/// popularity tags can: わたし is current and わたくし is not.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_bare_kanji_read_as_a_dead_reading_is_corrected() {
    let (tk, _) = setup();
    for text in ["私なら行けるはずだ", "私は知らない", "私が行く"] {
        assert_eq!(
            identity_of(&tokens_of(&tk, text), "私"),
            pair("私", "わたし"),
            "in {text}"
        );
    }
}

/// The correction is only for a reading Sudachi *guessed*. When the text spells
/// the word out, the reading is the text's and stands — わたくし written in kana
/// is わたくし, whatever the tags say about it.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_reading_the_text_spelled_out_is_never_corrected() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "わたくしが行きます");
    let t = tokens.first().expect("a token");
    assert_eq!(
        to_hiragana(&t.reading),
        "わたくし",
        "the text wrote it: {tokens:?}"
    );
}

/// Both readings of 何 are current, so the sentence decides and the tokenizer
/// must not. なに is the *preferred* reading here and なん still has to survive
/// — being acceptable is what protects it.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_word_with_two_current_readings_keeps_sudachis_choice() {
    let (tk, _) = setup();
    assert_eq!(
        identity_of(&tokens_of(&tk, "何を言っているんだ"), "何"),
        pair("何", "なん"),
        "Sudachi's reading, not the preferred one"
    );
}

/// A form is a form *of* something, so a stem must never be filed under a listed
/// word that cannot be inflected. Every one of these is a Sankoku headword, and
/// none of them is what the sentence said.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_stem_is_never_filed_under_a_word_that_does_not_conjugate() {
    let (tk, _) = setup();
    for (text, forbidden) in [
        ("許せない", "許せ"),
        ("やっておいた", "おいた"),
        ("もうなれた", "汝"),
        ("扉が開いて、続いて音がした", "続いて"),
        ("そうなんだ", "そうな"),
    ] {
        let bases: Vec<String> = tokens_of(&tk, text)
            .into_iter()
            .map(|t| t.base_form)
            .collect();
        assert!(
            !bases.contains(&forbidden.to_string()),
            "{text} must not yield {forbidden}: {bases:?}"
        );
    }
}

/// The same grammar has to give the same answer. 開いて and 続いて differ only in
/// whether Sankoku happens to list the string, which is not a fact about the
/// sentence.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn the_same_construction_is_analysed_the_same_way() {
    let (tk, _) = setup();
    let bases: Vec<String> = tokens_of(&tk, "扉が開いて、続いて音がした")
        .into_iter()
        .map(|t| t.base_form)
        .collect();
    assert!(bases.contains(&"開く".to_string()), "{bases:?}");
    assert!(bases.contains(&"続く".to_string()), "{bases:?}");
}

/// The repair, not just the refusal: なれ is a form of 慣れる, which conjugates,
/// so it is reached through the lemma's reading rather than the stem's.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn an_inflected_kana_verb_reaches_its_kanji_lemma() {
    let (tk, _) = setup();
    assert_eq!(
        identity_of(&tokens_of(&tk, "もうなれた"), "なれ"),
        pair("慣れる", "なれる")
    );
}

/// The master lists それは — the intensifier of 「それはもう見事に」 — and
/// ものを, and rejoining on that listing alone credits every それ + は and
/// もの + を to the expression. `NEVER_JOIN` names the ones that are phrases in
/// practice; the rest of what the join builds has to survive it.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_listed_expression_on_the_never_join_list_stays_apart() {
    let (tk, _) = setup();

    for (text, phrase) in [("それは私の本だ", "それは"), ("ものを見た", "ものを")]
    {
        let tokens = tokens_of(&tk, text);
        assert!(
            !tokens.iter().any(|t| t.surface == phrase),
            "{phrase} must not rejoin in {text}: {:?}",
            identities(&tokens)
        );
    }

    // The list refuses named strings and nothing else.
    let tokens = tokens_of(&tk, "本当に嬉しい");
    assert_eq!(
        identity_of(&tokens, "本当に"),
        pair("本当に", "ほんとうに"),
        "an expression not on the list still rejoins"
    );
}

/// 信じ is Sudachi's 信じる normalised, but read off its dictionary form 信ずる,
/// so the pair offered is (信じる, しんずる) — which the master does not list.
/// The ladder used to fall through to 信ずる, a spelling the text never used.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_normalised_spelling_keeps_its_own_reading() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "まるで信じられない");
    assert_eq!(identity_of(&tokens, "信じ"), pair("信じる", "しんじる"));
}

/// And only where the surface still sounds like it. お前 reads おまえ, 御前
/// reads ごぜん: two words sharing a normalisation, and swapping the spelling
/// would rewrite the sentence.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_normalisation_that_changes_the_sound_is_not_followed() {
    let (tk, _) = setup();
    for (text, surface, term, reading) in [
        ("お前は誰だ", "お前", "お前", "おまえ"),
        ("まだ来ない", "まだ", "まだ", "まだ"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert_eq!(identity_of(&tokens, surface), pair(term, reading), "{text}");
    }
}

/// A bound kanji is a different word that shares the spelling, and it is read
/// the other way *because* it is bound. The popularity dictionary scored the
/// free-standing word, so the reading correction has no business here: 数名 is
/// メイ whatever JMdict thinks of 名/な.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_bound_kanji_keeps_the_reading_its_binding_gives_it() {
    let (tk, _) = setup();
    for (text, surface, term, reading) in [
        ("生徒数名の名前", "名", "名", "めい"),
        ("被害者が死んだ", "者", "者", "しゃ"),
        ("練習生と探した", "生", "生", "せい"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert_eq!(identity_of(&tokens, surface), pair(term, reading), "{text}");
    }
}

/// One mora of kana spells nothing. Japanese has a kanji for every one of them,
/// so a normalisation onto one is always available and never evidence: UniDic
/// sends the か of 何もかも to the archaic pronoun 彼, the nominalising み of
/// 哀しみ to the homograph 味, and the honorific お to 御. Each is a pair Sankoku
/// lists, and none is the word anyone read.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn one_mora_of_kana_never_becomes_a_kanji_word() {
    let (tk, _) = setup();
    for (text, surface) in [
        ("何もかもが手遅れだった", "か"),
        ("今お茶を入れますね", "お"),
        ("ご家族は心配している", "ご"),
    ] {
        let tokens = tokens_of(&tk, text);
        let (term, _) = identity_of(&tokens, surface);
        assert_eq!(
            term, surface,
            "{text}: {surface} must keep its own spelling"
        );
    }
}

/// たらしい is the suffix of 憎たらしい and never the hearsay らしい after a past
/// tense. Sudachi segments 襲われ + た + らしい correctly; the join is what was
/// wrong, and it is lexical — hence the list, not a rule.
#[test]
#[ignore = "requires Sudachi dictionary (set JP_TOOLS_SUDACHI_DICT_PATH)"]
fn a_past_tense_before_hearsay_is_not_the_suffix_of_a_word() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "若者が襲われたらしい");
    let bases: Vec<&str> = tokens.iter().map(|t| t.base_form.as_str()).collect();
    assert!(!bases.contains(&"たらしい"), "{bases:?}");
    assert!(bases.contains(&"らしい"), "{bases:?}");
}
