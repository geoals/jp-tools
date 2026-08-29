//! What identity a token gets: the `(headword, reading)` pair the ledger keys
//! on, and whether it counts.
//!
//! The pairs in `master_pairs.tsv` are Sankoku's own, so a failure means the
//! tokenizer changed, not that the dictionary did.
//!
//! `KOTODEX_SUDACHI_DICT_PATH=$PWD/../system_full.dic \
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
        // 砂漠 outranks both verbs, which is exactly why the reading-only
        // fallback has to drop it before arbitrating on an inflected token.
        ("砂漠", "さばく", 4872),
        ("裁く", "さばく", 11982),
        ("捌く", "さばく", 13939),
    ]
    .into_iter()
    .map(|(t, r, n)| ((t.to_string(), r.to_string()), n))
    .collect()
}

/// Jiten's rank per spelling, for the short-kana guard. The production numbers,
/// because the guard turns on magnitude: 時 and 筈 are spellings the language
/// uses at two morae, 弥 and 滓 are not.
fn reader_ranks() -> HashMap<String, i64> {
    [
        ("時", 275),
        ("筈", 257),
        ("後", 130),
        ("弥", 36070),
        ("滓", 19422),
        ("箒", 22217),
        ("伺う", 6482),
        ("窺う", 12180),
        // 母 is in this work's cast list and common in fiction, which is the
        // whole reason the cast has a frequency veto on it.
        ("母", 872),
        ("凛", 9368),
        // A character called ココ, and ここ is a word every page uses — the
        // pair the veto would fire on if the fold ran before the name gate.
        ("ここ", 76),
        ("ロブ", 44139),
        ("出雲", 40106),
        // Jiten's own numbers: the compound is a word the reader meets whole,
        // and both halves are rarer than it.
        ("宣戦布告", 11761),
        ("宣戦", 47197),
        ("布告", 34041),
    ]
    .into_iter()
    .map(|(t, n)| (t.to_string(), n))
    .collect()
}

/// What `dictionaries::preferred_readings` derives for these words from
/// Jitendex and BCCWJ; that derivation has its own tests.
fn preferences() -> HashMap<String, PreferredReading> {
    [
        ("私", "わたし", vec!["わたし", "あたし", "あたくし"]),
        ("何", "なに", vec!["なに", "なん"]),
        // JMdict scores the free-standing noun of each of these and the bound
        // reading zero, which is what the bound-morpheme guard has to refuse.
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
        "捌く",
        "裁く",
        "潜める",
        "深い",
        "煩い",
        "凄い",
        "臭い",
        "危ない",
        "面白い",
        "旨い",
        "上手い",
        "会う",
        "遭う",
        "上る",
        "登る",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn setup() -> (SudachiTokenizer, MasterWords) {
    build(HashSet::new())
}

/// The same tokenizer, told who the cast are — `work_names`, which is the only
/// term-level answer to whether a token is a name.
fn with_cast(names: &[&str]) -> SudachiTokenizer {
    build(names.iter().map(|n| n.to_string()).collect()).0
}

fn build(names: HashSet<String>) -> (SudachiTokenizer, MasterWords) {
    let dict_path =
        std::env::var("KOTODEX_SUDACHI_DICT_PATH").expect("KOTODEX_SUDACHI_DICT_PATH must be set");
    let entries = master_entries();
    let lexicon: HashSet<String> = entries.iter().map(|(t, _)| t.clone()).collect();
    // A non-empty deck is what puts the tokenizer on the C→B→A path, which is
    // the one production runs.
    let tokenizer = SudachiTokenizer::new(Path::new(&dict_path), HashSet::from(["x".to_string()]))
        .unwrap()
        .with_lexicon(lexicon.clone())
        .with_master_readings(&entries)
        .with_frequency(ranks())
        .with_reader_frequency(std::sync::Arc::new(reader_ranks()))
        .with_preferred_readings(preferences())
        .with_conjugatable(conjugatable())
        .with_names(names);
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
/// んっと normalises to うんと — a real Sankoku headword.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_normalized_spelling_the_master_does_not_list_falls_back_to_the_written_one() {
    let (tk, master) = setup();

    // 一寸 is listed, but only as いっすん.
    let tokens = tokens_of(&tk, "ちょっと待って");
    assert_eq!(
        identity_of(&tokens, "ちょっと"),
        pair("ちょっと", "ちょっと")
    );
    assert_eq!(identity_of(&tokens, "待っ"), pair("待つ", "まつ"));

    // 為る likewise: without the fallback the commonest verb in the language is
    // off the master scale.
    let tokens = tokens_of(&tk, "そうすると決めた");
    assert_eq!(identity_of(&tokens, "する"), pair("する", "する"));
    let suru = tokens.iter().find(|t| t.surface == "する").unwrap();
    assert!(counts_as_word(suru, &master));
}

/// A potential form: Sudachi hands back the base verb's spelling with the
/// potential's reading, and (行く, いける) is a pair no dictionary has. Asking
/// what 行く reads as on its own repairs it.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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

#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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

/// The same rule inside the reading-only fallback, where the refusal alone is
/// not enough: さばい reads さばく, and the commonest headword with that reading
/// is 砂漠. Dropping the nouns has to leave the verbs to be arbitrated between,
/// or a conjugated verb ends up filed under a noun.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_reading_only_match_on_an_inflected_token_stays_a_verb() {
    let (tk, _) = setup();
    let (term, _) = identity_of(&tokens_of(&tk, "魚をさばいている。"), "さばい");
    assert!(term == "捌く" || term == "裁く", "{term}");
}

/// An expression whose tail the identity ladder respells. ひそめ is filed under
/// 潜める, so the run's canonical spelling is 眉を潜める — not a headword, while
/// 眉をひそめる is one. The join has to try the tail as the text spelt it.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_expression_is_found_under_the_spelling_the_text_used() {
    let (tk, _) = setup();
    let ids = identities(&tokens_of(&tk, "彼は眉をひそめた。"));
    assert!(ids.iter().any(|(t, _)| t == "眉をひそめる"), "{ids:?}");
}

/// The same grammar has to give the same answer. 開いて and 続いて differ only in
/// whether Sankoku happens to list the string, which is not a fact about the
/// sentence.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_listed_expression_on_the_never_join_list_stays_apart() {
    let (tk, _) = setup();

    for (text, phrase) in [
        ("それは私の本だ", "それは"),
        ("ものを見た", "ものを"),
        // Both readings open a clause, so position cannot arbitrate it and the
        // list has to: it is the place far more often than the conjunction.
        ("そこでシェリーが声をあげた", "そこで"),
    ] {
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

/// ところで and すると are the conjunction where they open a clause and the
/// plain words anywhere else: 「離れたところで」 is a place, and 「油断すると」
/// is the verb and the conditional, where building the expression also swallows
/// the する.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_expression_that_is_only_a_word_at_a_clause_start_is_refused_mid_clause() {
    let (tk, _) = setup();

    for (text, phrase) in [
        ("離れたところで待っていた", "ところで"),
        ("油断すると大変だ", "すると"),
        ("それはそれで合理的な考えだ", "それで"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert!(
            !tokens.iter().any(|t| t.surface == phrase),
            "{phrase} must not rejoin mid-clause in {text}: {:?}",
            identities(&tokens)
        );
    }

    // And is still built where the sentence opens on it.
    for (text, phrase) in [
        ("ところで、話は変わるが", "ところで"),
        ("すると、扉が開いた", "すると"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert!(
            tokens.iter().any(|t| t.surface == phrase),
            "{phrase} must rejoin at a clause start in {text}: {:?}",
            identities(&tokens)
        );
    }
}

/// 「〜ないと思う」 is ない and a quotative と, not the ないと that means "must",
/// and building the expression takes 思う's complement marker with it.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_expression_is_refused_where_the_next_word_quotes_it() {
    let (tk, _) = setup();

    for text in ["名前は出ないと思う", "信じられないという感じだ"] {
        let tokens = tokens_of(&tk, text);
        assert!(
            !tokens.iter().any(|t| t.surface == "ないと"),
            "ないと must not rejoin before a quoting verb in {text}: {:?}",
            identities(&tokens)
        );
    }

    // The construction it exists for is untouched.
    let tokens = tokens_of(&tk, "早く逃げないといけない");
    assert!(
        tokens.iter().any(|t| t.surface == "ないと"),
        "{:?}",
        identities(&tokens)
    );
}

/// 「巨大なものの前で」 is もの and a genitive の with 前 hanging off it, not the
/// concessive ものの. What comes before separates nothing — た is on both sides
/// of it — and what follows does.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_expression_is_refused_where_the_next_word_hangs_off_its_genitive() {
    let tk = with_cast(&[]);

    for text in ["巨大なものの前でもがく", "最近書かれたもののようだ"] {
        let tokens = tokens_of(&tk, text);
        assert!(
            !tokens.iter().any(|t| t.surface == "ものの"),
            "ものの must not rejoin before a noun in {text}: {:?}",
            identities(&tokens)
        );
    }

    let tokens = tokens_of(&tk, "口では抵抗しているものの、暴れない");
    assert!(
        tokens.iter().any(|t| t.surface == "ものの"),
        "{:?}",
        identities(&tokens)
    );
}

/// 確かに and 本当に are both a noun-ish word plus に and both Sankoku headwords,
/// and the two are indistinguishable to a reader: Sudachi calls 本当's に a case
/// particle and 確か's the copula's 連用形, so the no-inflected-part rule would
/// refuse 確かに alone.
///
/// ように is the shape that rule exists for, and every dictionary here lists it —
/// so only `NEVER_JOIN` reaches it.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn the_adverbial_of_a_na_adjective_is_one_word_where_the_master_lists_it() {
    let (tk, _) = setup();

    let tokens = tokens_of(&tk, "確かにあそこは静かだ");
    assert_eq!(identity_of(&tokens, "確かに"), pair("確かに", "たしかに"));

    let tokens = tokens_of(&tk, "泣いているように見えた");
    assert!(
        !tokens.iter().any(|t| t.surface == "ように"),
        "{:?}",
        identities(&tokens)
    );
}

/// Sudachi keeps 待ちたまえ whole only at the end of the input; anything after it
/// and the lattice cuts た + まえ, with まえ keyed on 前. Nothing downstream
/// rejoins it — a run opening on the auxiliary た is a function word, which a
/// standard dictionary may not license — so the cut list is the only lever.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_boundary_inside_tamae_is_cut_before_sudachi_sees_it() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "待ちたまえ！");

    assert_eq!(identity_of(&tokens, "たまえ"), pair("給う", "たまう"));
}

/// 「そういえばあそこで」 is "speaking of which" — Sankoku's own そう言えば — and
/// not the conditional of そういう, which is what the conjugated-tail path builds
/// out of そう + いえ. No path reaches そう言えば, so the join is refused and what
/// is left is そう + 言う + ば.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_expression_is_refused_where_the_master_lists_its_conditional() {
    let (tk, _) = setup();

    let tokens = tokens_of(&tk, "そういえばあそこで泣いてた");
    assert!(
        !tokens.iter().any(|t| t.base_form == "そういう"),
        "そういう must not be built over そういえば: {:?}",
        identities(&tokens)
    );

    let tokens = tokens_of(&tk, "そういう事もある");
    assert!(
        tokens.iter().any(|t| t.base_form == "そういう"),
        "{:?}",
        identities(&tokens)
    );
}

/// 「ヒロちゃんと友だちになりたい」 is a name, its honorific ちゃん and the
/// comitative と — not the adverb ちゃんと.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_expression_is_refused_where_a_name_makes_its_first_part_an_honorific() {
    let tk = with_cast(&["ヒロ"]);

    let tokens = tokens_of(&tk, "ヒロちゃんと友だちになりたい");
    assert!(
        !tokens.iter().any(|t| t.surface == "ちゃんと"),
        "ちゃんと must not rejoin after a name: {:?}",
        identities(&tokens)
    );

    let tokens = tokens_of(&tk, "ちゃんと確認してから来る");
    assert!(
        tokens.iter().any(|t| t.surface == "ちゃんと"),
        "{:?}",
        identities(&tokens)
    );
}

/// The same list read the other way. でも and だが are two kana, so the join's
/// length floor would refuse them everywhere, including the clause openings
/// where they are the word — while 「読んでも」 and 「一人でも」 are で + も and
/// must stay apart, which is the floor doing its job.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_two_kana_conjunction_is_built_where_it_opens_a_clause() {
    let (tk, _) = setup();

    for (text, phrase) in [("でも、それは違う", "でも"), ("だが俺は言う", "だが")]
    {
        let tokens = tokens_of(&tk, text);
        assert!(
            tokens.iter().any(|t| t.surface == phrase),
            "{phrase} must rejoin at a clause start in {text}: {:?}",
            identities(&tokens)
        );
    }

    for (text, phrase) in [("本を読んでも分からない", "でも"), ("良い考えだが", "だが")]
    {
        let tokens = tokens_of(&tk, text);
        assert!(
            !tokens.iter().any(|t| t.surface == phrase),
            "{phrase} must not rejoin mid-clause in {text}: {:?}",
            identities(&tokens)
        );
    }
}

/// A normalisation that swaps one kanji for another has changed which word is
/// being claimed, and where the text's own spelling is a headword too there is
/// nothing to weigh: 検死 is what the page said and 検屍 is a different string
/// the ledger would key on. The reader saw one of them.
///
/// Free, because the surface is on the master scale as written — which is also
/// the fence. An inflected stem is not a word, so it is answered by the rung
/// below instead.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_normalisation_may_not_drop_a_kanji_the_text_wrote() {
    let (tk, _) = setup();
    for (text, surface, reading) in [
        ("綺麗に整頓されている", "綺麗", "きれい"),
        ("偽装工作だ", "偽装", "ぎそう"),
        ("詳しい検死ができたら", "検死", "けんし"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert_eq!(
            identity_of(&tokens, surface),
            pair(surface, reading),
            "{text}"
        );
    }
}

/// The other half of that class: an inflected surface, where refusing the swap
/// would leave a stem rather than a word. 上手く is not a word and 上手い is;
/// 遭っ is not and 遭う is. Both are the master's own spellings of the reading
/// Sudachi gave the token, and only the kanji on the page separates them.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_inflected_stem_keeps_its_kanji_through_the_masters_other_spelling() {
    let (tk, _) = setup();
    for (text, surface, term, reading) in [
        ("上手くいくわけがない", "上手く", "上手い", "うまい"),
        ("辛い目に遭った", "遭っ", "遭う", "あう"),
        // Read off the potential 登れる, so the pair is (上る, のぼれる) and no
        // master reading is のぼれる — the swapped spelling's own reading has to
        // be asked instead.
        ("高い塀は登れないと思う", "登れ", "登る", "のぼる"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert_eq!(identity_of(&tokens, surface), pair(term, reading), "{text}");
    }
}

/// 信じ is Sudachi's 信じる normalised, but read off its dictionary form 信ずる,
/// so the pair offered is (信じる, しんずる) — which the master does not list.
/// Falling through to 信ずる would spell the word a way the text never did.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_normalised_spelling_keeps_its_own_reading() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "まるで信じられない");
    assert_eq!(identity_of(&tokens, "信じ"), pair("信じる", "しんじる"));
}

/// And only where the surface still sounds like it. お前 reads おまえ, 御前
/// reads ごぜん: two words sharing a normalisation, and swapping the spelling
/// would rewrite the sentence.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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

/// The kana alphabet is part of the spelling. Sudachi normalises ザル onto ざる
/// and マジ onto まじ, and the master lists each pair as two words: the colander
/// against the classical negative, the slang against the classical negative
/// again. Only the fold that is nothing *but* the alphabet is refused — サクラ
/// still normalises onto 桜 — and only where the master reads the katakana
/// headword in hiragana, since a katakana entry read in katakana is a loanword
/// (モノ is monochrome) and a line writing モノ means 物.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_katakana_word_is_not_folded_onto_its_hiragana_homophone() {
    let (tk, _) = setup();
    for (text, surface, term, reading) in [
        ("ザルで水を切る", "ザル", "ザル", "ざる"),
        ("マジで言っている", "マジ", "マジ", "まじ"),
        ("サクラが咲いた", "サクラ", "桜", "さくら"),
        ("モノは言いよう", "モノ", "もの", "もの"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert_eq!(identity_of(&tokens, surface), pair(term, reading), "{text}");
    }
}

/// The other half of that: where the master lists **only** the hiragana, the
/// katakana is not a spelling of anything and the line means the word. Without
/// the fold ウチ and コイツ get ledger rows of their own beside うち and こいつ.
///
/// A katakana headword in its own right is untouched, because the fold is last
/// and can only win where nothing the text wrote is listed.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn katakana_the_master_does_not_list_folds_onto_the_hiragana_it_does() {
    let (tk, _) = setup();
    for (text, surface, term, reading) in [
        ("ウチが必要だ", "ウチ", "うち", "うち"),
        ("コイツは誰だ", "コイツ", "こいつ", "こいつ"),
        ("スマホを見た", "スマホ", "スマホ", "すまほ"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert_eq!(identity_of(&tokens, surface), pair(term, reading), "{text}");
    }
}

/// 〜あい and 〜おい contract to 〜えー in speech, and Sudachi reads all of them
/// right except where the spelling it normalises onto carries a second reading:
/// 煩い is わずらい as well as うるさい, and うるせー takes the noun.
///
/// The un-contraction only ever offers a reading the master already lists for
/// that spelling, so the family that is already right stays right — and a word
/// simply spelt the way it is read is untouched, since a kana headword matches
/// on the headword alone and へえ would have taken はい.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_colloquial_adjective_ending_names_the_reading_it_contracts_from() {
    let (tk, _) = setup();
    for (text, surface, term, reading) in [
        ("うるせーぞ黙れ", "うるせー", "煩い", "うるさい"),
        ("うるせえって言ってんだろ", "うるせえ", "煩い", "うるさい"),
        ("うるせぇんだよ", "うるせぇ", "煩い", "うるさい"),
        ("すげえ量の本だ", "すげえ", "凄い", "すごい"),
        ("この部屋くせえな", "くせえ", "臭い", "くさい"),
        ("あぶねーから下がれ", "あぶねー", "危ない", "あぶない"),
        ("へえ、そうなんだ", "へえ", "へえ", "へえ"),
    ] {
        let tokens = tokens_of(&tk, text);
        assert_eq!(identity_of(&tokens, surface), pair(term, reading), "{text}");
    }
}

/// The same word said the other way: うっさい swallows うるさい's る into a small
/// っ rather than holding the vowel, so no ending can be un-contracted and the
/// reading has to be read off the master's own list for that spelling.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_mora_swallowed_into_a_sokuon_names_the_reading_it_came_from() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "うっさいな、黙れ");

    assert_eq!(identity_of(&tokens, "うっさい"), pair("煩い", "うるさい"));
}

/// The fold has to ask the cast list itself, not leave the name to the gate
/// downstream. That gate vetoes a cast name common enough to be an ordinary
/// word and it asks the *identity*, so folding first makes the veto fire on the
/// fold: ココ becomes ここ, stops being a name, and hands a character's every
/// sighting to the pronoun.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_katakana_cast_name_is_never_folded_onto_the_word_it_spells() {
    let tk = with_cast(&["ココ"]);
    let tokens = tokens_of(&tk, "ココは黙って頷いた");

    assert_eq!(identity_of(&tokens, "ココ"), pair("ココ", "ここ"));
    assert!(
        tokens.iter().any(|t| t.surface == "ココ" && t.proper_noun),
        "{tokens:?}"
    );
}

/// A bound kanji is a different word that shares the spelling, and it is read
/// the other way *because* it is bound. The popularity dictionary scored the
/// free-standing word, so the reading correction has no business here: 数名 is
/// メイ whatever JMdict thinks of 名/な.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
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
/// tense. Sudachi segments 襲われ + た + らしい correctly; the error is the join's,
/// and it is lexical — hence the list, not a rule.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_past_tense_before_hearsay_is_not_the_suffix_of_a_word() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "若者が襲われたらしい");
    let bases: Vec<&str> = tokens.iter().map(|t| t.base_form.as_str()).collect();
    assert!(!bases.contains(&"たらしい"), "{bases:?}");
    assert!(bases.contains(&"らしい"), "{bases:?}");
}

/// The explanation and the tokenization are one code path, and this is what
/// says so. `explain` is only [`Tokenizer::tokenize`] with the recorder on; the
/// day it becomes a second implementation of the rules it starts explaining a
/// pipeline nobody runs, which is worse than explaining nothing.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn explaining_a_line_yields_the_tokens_tokenizing_it_does() {
    let (tk, _) = setup();
    for line in [
        "そのメルルの胸を、とんっと軽く押した。",
        "彼女はしゃくりあげながら話した。",
        "音だったそうです",
        "それは私の本だ",
        "ごめんなさいっ",
    ] {
        let (explained, steps) = tk.explain(line).unwrap();
        assert_eq!(explained, tokens_of(&tk, line), "{line}");
        assert!(!steps.is_empty(), "no steps recorded for {line}");
    }
}

/// Two morae of kana name a rare kanji word by coincidence, never by evidence.
///
/// 居やしない is the emphatic negative of いる, so the correct split has no いや
/// in it; Sudachi's does, and 弥 is a master headword reading いや, so the pair
/// matches exactly and a rare adverb enters the ledger off one encounter. The
/// guard is the one-mora rule at the next mora out — a two-kana string is
/// homophonous with a dozen rare entries, so the match is found every time and
/// means nothing about any of them.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn two_morae_of_kana_never_name_a_spelling_nobody_writes() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "ひとりもいやしないんだ");

    let bases: Vec<&str> = tokens.iter().map(|t| t.base_form.as_str()).collect();
    assert!(!bases.contains(&"弥"), "{bases:?}");
}

/// The rarity fence has one exception, and its own class carries it: an
/// interjection is never とき or はず. はは is laughter and takes 母, ひっ takes
/// the prefix 引っ — both far too common for the rank to refuse.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_two_mora_cry_never_names_a_kanji_word() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "あははは、はは、おかしい");

    let bases: Vec<&str> = tokens.iter().map(|t| t.base_form.as_str()).collect();
    assert!(!bases.contains(&"母"), "{bases:?}");
}

/// And the rarity is half the rule, not decoration: とき, あと and はず are the
/// same two morae, and 時, 後 and 筈 are simply what they are. A guard keyed on
/// length alone would take the commonest words in the language off the scale.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn two_morae_of_kana_still_name_a_spelling_the_language_uses() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "そのときが来た");

    assert_eq!(identity_of(&tokens, "とき"), pair("時", "とき"));
}

/// Katakana is exempt, because katakana is itself the decision. A word gets
/// written ハエ, キク, ツタ, カス or アザ precisely because its kanji is one
/// nobody reads, so the rare spelling is the right answer there and the guard
/// would throw away exactly the words it exists to protect.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn katakana_may_still_name_a_rare_spelling() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "中身はカスだ");

    assert_eq!(identity_of(&tokens, "カス"), pair("滓", "かす"));
}

/// Three morae is where the coincidence stops and the evidence starts: ほうき
/// is 箒, a rare spelling and the right one, which is why the guard stops at two.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn three_morae_of_kana_may_name_a_rare_spelling() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "長い柄のほうきがある");

    assert_eq!(identity_of(&tokens, "ほうき"), pair("箒", "ほうき"));
}

/// A join made on the reading may not overwrite a kanji the text wrote.
///
/// 生誕祭 and 聖誕祭 are both せいたんさい, and only the second is a master
/// headword, so the reading path would rewrite a birthday celebration into
/// Christmas. The kanji on the page is evidence and the reading is not: where
/// the two disagree the page wins, which is the same rule the identity ladder
/// applies when it refuses to add kanji nobody wrote.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_sounded_join_never_replaces_a_kanji_the_text_wrote() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "生誕祭の準備をする");

    let bases: Vec<&str> = tokens.iter().map(|t| t.base_form.as_str()).collect();
    assert!(!bases.contains(&"聖誕祭"), "{bases:?}");
}

/// A mimetic is written in either kana, and the dictionary picked one.
///
/// スッ + と spells no headword, so the run comes apart: と is left free, joins
/// する into とする, and the mimetic is gone from the line. Sankoku lists すっと;
/// only the alphabet differs.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_katakana_mimetic_joins_the_hiragana_headword_it_sounds_like() {
    let (tk, _) = setup();
    let tokens = tokens_of(&tk, "胸がスッとする");

    let joined = tokens
        .iter()
        .find(|t| t.surface == "スッと")
        .unwrap_or_else(|| panic!("スッ + と must join: {tokens:?}"));
    assert_eq!(joined.base_form, "すっと");
    // And the と is then no longer free to be taken by とする.
    let bases: Vec<&str> = tokens.iter().map(|t| t.base_form.as_str()).collect();
    assert!(!bases.contains(&"とする"), "{bases:?}");
}

/// SudachiDict tags a handful of everyday expressions 固有名詞, and the
/// highlighter drops every proper noun before it consults the ledger — so
/// 断腸の思い and 机上の空論 are invisible however often they are read.
///
/// Mixed script is what separates them from the cast, and being a master
/// headword is not: 橘, 出雲, 葵, 司 and シェリー are all master headwords and
/// all names. A Japanese name does not carry okurigana.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_master_headword_written_with_okurigana_is_not_a_name() {
    let (tk, _) = setup();

    let tokens = tokens_of(&tk, "少女は断腸の思いでボタンを押した");
    let idiom = tokens
        .iter()
        .find(|t| t.surface == "断腸の思い")
        .unwrap_or_else(|| panic!("断腸の思い must be one token: {tokens:?}"));
    assert!(!idiom.proper_noun, "{idiom:?}");

    // And the cast stays gated, which is the whole point of the gate. シェリー
    // is a master headword too — it is sherry — and it carries no okurigana.
    let tokens = tokens_of(&tk, "私は橘シェリーっていいますっ");
    let name = tokens.iter().find(|t| t.surface == "シェリー").unwrap();
    assert!(name.proper_noun, "{name:?}");
}

/// Sudachi has no entry for most of a VN's cast, so it does not merely leave
/// the name untagged — it *splits* it, and both halves are ordinary words the
/// ledger then counts forever.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_name_the_cast_list_holds_is_one_token_and_a_name() {
    let tk = with_cast(&["世凪"]);
    let tokens = tokens_of(&tk, "名前は世凪。");
    let name = tokens
        .iter()
        .find(|t| t.surface == "世凪")
        .unwrap_or_else(|| panic!("世凪 must be one token: {tokens:?}"));
    assert!(name.proper_noun, "{name:?}");
    assert_eq!(name.base_form, "世凪");
}

/// The verdict is a fact about the term, not about the sentence. Sudachi's
/// 固有名詞 is a per-occurrence tag, so the same name comes out
/// `excluded: "name"` in one sentence and vocabulary in the next.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_cast_name_is_a_name_in_every_sentence() {
    let tk = with_cast(&["ロブ"]);
    for line in ["おいウィル、ロブを殴れ。", "さっきのロブの話……"] {
        let token = tokens_of(&tk, line)
            .into_iter()
            .find(|t| t.surface == "ロブ")
            .unwrap_or_else(|| panic!("no ロブ in {line}"));
        assert!(token.proper_noun, "{line}: {token:?}");
    }
}

/// A name Sudachi cannot account for comes back glued to whatever follows it:
/// 「凛とオリヴィア」 analyses as the adverb 凛と.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_name_glued_to_a_particle_comes_apart() {
    let tk = with_cast(&["凛"]);
    let surfaces: Vec<String> = tokens_of(&tk, "凛とオリヴィアが並んでいる")
        .into_iter()
        .map(|t| t.surface)
        .collect();
    assert!(surfaces.contains(&"凛".to_string()), "{surfaces:?}");
    assert!(!surfaces.contains(&"凛と".to_string()), "{surfaces:?}");
}

/// And only where what is left over is grammar: ウィルス is a word with a name
/// inside it, and 出雲大社 is a word every dictionary lists.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_word_that_merely_starts_with_a_name_stays_whole() {
    let tk = with_cast(&["ウィル", "出雲"]);
    for (line, word) in [
        ("ウィルスに感染した", "ウィルス"),
        ("出雲大社に行った", "出雲"),
    ] {
        let surfaces: Vec<String> = tokens_of(&tk, line)
            .into_iter()
            .map(|t| t.surface)
            .collect();
        assert!(surfaces.contains(&word.to_string()), "{line}: {surfaces:?}");
    }
}

/// A cast name common enough to be an ordinary word is the word. VNDB lists 母
/// as a character of this very work, and it is one of the commonest words in
/// fiction.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn an_everyday_word_in_the_cast_list_is_still_a_word() {
    let tk = with_cast(&["母"]);
    let token = tokens_of(&tk, "母の遺した車")
        .into_iter()
        .find(|t| t.surface == "母")
        .unwrap();
    assert!(!token.proper_noun, "{token:?}");
}

/// Sudachi reads a bare 深い as ブカイ and a bare 箱 as バコ — the readings
/// 奥深い and 靴箱 give them — so the pair is one no dictionary lists and the
/// word falls off the master scale.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_word_standing_alone_loses_the_compounds_voicing() {
    let (tk, master) = setup();
    for (line, surface, want) in [
        ("海よりも深い", "深い", "ふかい"),
        ("傷は深かったのだ", "深かっ", "ふかい"),
    ] {
        let token = tokens_of(&tk, line)
            .into_iter()
            .find(|t| t.surface == surface)
            .unwrap_or_else(|| panic!("no {surface} in {line}"));
        assert_eq!(to_hiragana(&token.reading), want, "{line}");
        assert!(master.lists(&token.base_form, &token.reading), "{line}");
    }
}

/// But only the voicing. 所為 is せい in the text and しょい in the master, and
/// those are two words rather than one word's compound form — rewriting the
/// reading there asserts a word nobody read.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_reading_the_master_merely_lacks_is_left_alone() {
    let (tk, _) = setup();
    let token = tokens_of(&tk, "私が生まれた所為で起きた")
        .into_iter()
        .find(|t| t.surface == "所為")
        .unwrap();
    assert_eq!(to_hiragana(&token.reading), "せい", "{token:?}");
}

/// Mode C hands 宣戦布告 over as one morpheme and only Jitendex lists it, so
/// the gate would break an everyday word into two rarer ledger rows. A
/// compound the reader-facing list ranks above both its halves is one word.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_compound_commoner_than_its_halves_is_not_split() {
    let (tk, _) = setup();
    let surfaces: Vec<String> = tokens_of(&tk, "国に対する宣戦布告に等しい")
        .into_iter()
        .map(|t| t.surface)
        .collect();
    assert!(surfaces.contains(&"宣戦布告".to_string()), "{surfaces:?}");
}

/// The lattice picks the cheapest path over the whole line, and where a word
/// Sudachi lacks sits next to one it has, that path can run straight through
/// the boundary: なんてひどい comes back as なん + てひどい, and 手酷い is a real
/// Sankoku headword reading てひどい, so every rule after the segmentation
/// confirms it.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_word_the_lattice_ran_past_the_end_of_is_cut_back() {
    let (tk, _) = setup();
    for (line, want, gone) in [
        ("なんてひどい怪我なんだ", "なんて", "てひどい"),
        ("またいちからやり直しだ", "また", "たいち"),
        ("イギリスには牛乳粥があった", "牛乳", "乳粥"),
    ] {
        let surfaces: Vec<String> = tokens_of(&tk, line)
            .into_iter()
            .map(|t| t.surface)
            .collect();
        assert!(surfaces.contains(&want.to_string()), "{line}: {surfaces:?}");
        assert!(
            !surfaces.contains(&gone.to_string()),
            "{line}: {surfaces:?}"
        );
    }
}

/// And nowhere else. Cutting the line costs the lattice its context, so the
/// pass asks the *analysis* whether the boundary came out wrong rather than
/// asking the string whether it is present — また lies wholly inside たまたま
/// and 跨いで, and crosses the まま | ただ of 「縛られたままただ」 by accident.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_word_that_merely_contains_one_is_left_alone() {
    let (tk, _) = setup();
    for (line, want) in [
        ("本当にたまたまだったな", "たまたま"),
        ("三つの季節をまたいで作った", "またい"),
        ("後ろ手に縛られたままただ待った", "まま"),
        ("学校なんて通ってんだ", "通っ"),
    ] {
        let surfaces: Vec<String> = tokens_of(&tk, line)
            .into_iter()
            .map(|t| t.surface)
            .collect();
        assert!(surfaces.contains(&want.to_string()), "{line}: {surfaces:?}");
    }
}

/// A 接頭辞 is the usual *first* half of the compound the reading join exists
/// to rebuild, and its kanji is evidence like any other: the fence is the
/// reading naming exactly one master headword, not the part-of-speech of the
/// head. 大 + アリ sounds like 大あり, ご + 愁傷 + さま like 御愁傷様, and 物 +
/// 足り (the stem of 物足りる) like 物足りる — Sudachi hands all three over in
/// pieces and the master has the whole word.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_prefix_compound_joins_when_its_reading_names_one_headword() {
    let (tk, _) = setup();
    for (line, want, want_identity) in [
        ("……大アリだ。", "大アリ", pair("大あり", "おおあり")),
        (
            "犯人はご愁傷さま～♪",
            "ご愁傷さま",
            pair("御愁傷様", "ごしゅうしょうさま"),
        ),
        ("物足りない", "物足り", pair("物足りる", "ものたりる")),
    ] {
        let tokens = tokens_of(&tk, line);
        let t = tokens
            .iter()
            .find(|t| t.surface == want)
            .unwrap_or_else(|| panic!("{line}: no token with surface {want}: {tokens:?}"));
        assert_eq!(
            (t.base_form.clone(), to_hiragana(&t.reading)),
            want_identity,
            "{line}"
        );
    }
}

/// A kana 接頭辞 alone is still not enough: お + 世話 sounds like お世話, but
/// the admission is kanji-gated like the rest of the reading path, and without
/// a kanji in the head every all-kana run is a guess — the same fence that
/// keeps そう + する off 相する and こと + し off 今年.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_kana_prefix_alone_is_not_enough_to_join() {
    let (tk, _) = setup();
    let surfaces: Vec<String> = tokens_of(&tk, "お世話になります")
        .into_iter()
        .map(|t| t.surface)
        .collect();
    assert!(!surfaces.contains(&"お世話".to_string()), "{surfaces:?}");
}

/// The one thing a sounded join may not do is overwrite a kanji the text
/// wrote. 最 + 低減 sounds like 最低限 and the master lists it, but the page
/// spelt 減 and the reading さいていげん belongs to a word that does not keep
/// it — so the run stays 最 + 低減 rather than asserting a spelling nobody
/// read. A real script does spell it this way.
#[test]
#[ignore = "requires Sudachi dictionary (set KOTODEX_SUDACHI_DICT_PATH)"]
fn a_sounded_join_may_not_respell_a_kanji_the_text_wrote() {
    let (tk, _) = setup();
    // 最低限 in the master is what makes the join *offer* 最低減, so the
    // refusal is what is being tested rather than an absent headword.
    let surfaces: Vec<String> = tokens_of(&tk, "最低減")
        .into_iter()
        .map(|t| t.surface)
        .collect();
    assert_eq!(surfaces, ["最", "低減"], "{surfaces:?}");

    let whole = tokens_of(&tk, "最低限");
    assert_eq!(whole.len(), 1, "{whole:?}");
    assert_eq!(identities(&whole), [pair("最低限", "さいていげん")]);
}
