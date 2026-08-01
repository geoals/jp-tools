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
fn ranks() -> HashMap<String, i64> {
    [("伺う", 1886), ("窺う", 2831), ("敵", 1039), ("隙", 4156)]
        .into_iter()
        .map(|(t, r)| (t.to_string(), r))
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
        .with_frequency(ranks());
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
