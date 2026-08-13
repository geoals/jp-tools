//! Run the real CompactDef call over a fixed probe set and print what comes
//! back. This is how a rubric change is judged — by the production path, cold,
//! at the effort it actually runs at.
//!
//!     cargo run -p jp-mine-core --example tag_probe
//!
//! Needs `JP_TOOLS_ANTHROPIC_API_KEY`. Costs one call per probe.
//!
//! The set is fixed so two runs are comparable. Each probe carries what it is
//! there to catch, never an expected answer: a fixture asserting the tier would
//! turn a judgement into a regression test, and the whole problem with this axis
//! is that nobody here knows the right answers.
//!
//! Two probes are marked TOLD — a native speaker reported not recognising them,
//! and that fact is in the repository now. They are a check that the tier is not
//! COMMON, and nothing more; anyone tuning against them is tuning against a
//! leak.

use jp_mine_core::compactdef;

/// `(target, sentence, what this probe is for)`. The sentence carries `<b>`
/// around the target, exactly as the mining path sends it.
const PROBES: &[(&str, &str, &str)] = &[
    (
        "二の句",
        "<b>二の句</b>がつげなかった。囁きは悪魔のようだった。",
        "TOLD not recognised — bookish set phrase, must not be COMMON",
    ),
    (
        "飛び道具",
        "相手は<b>飛び道具</b>を持っている。近づく前に仕留められるぞ。",
        "TOLD not recognised — print/game frequent, must not be COMMON",
    ),
    (
        "誤謬",
        "些細な<b>誤謬</b>であっても指摘してみるべきか。",
        "stiff but sayable — FORMAL without LITERARY",
    ),
    (
        "束の間",
        "夢だったのだと安堵したのも<b>束の間</b>、その眼が見開かれていく。",
        "plain register, writing-heavy — PLAIN · LITERARY must be reachable",
    ),
    (
        "悄然",
        "彼は<b>悄然</b>と肩を落として立ち去った。",
        "stiff AND writing-only — FORMAL · LITERARY must be reachable",
    ),
    (
        "陰嚢",
        "細い指先が<b>陰嚢</b>と竿とを扱き上げて、クマを嬲った。",
        "the old missing-baseline card — a mark must not replace the baseline",
    ),
    (
        "でかい",
        "「そいつは<b>でかい</b>犬だな」",
        "blunt but not slang — the case CASUAL was added for",
    ),
    (
        "けったい",
        "そんな<b>けったい</b>なのとドッキングしないでくださいっ",
        "dialect — no population claim is available, so no tier",
    ),
    (
        "実装",
        "「その機能の<b>実装</b>は来週だ」",
        "trade vocabulary — TECHNICAL beside a baseline",
    ),
    (
        "ナメクジ",
        "<b>ナメクジ</b>の這った後みたいな……。",
        "childhood word — CORE should still be reachable",
    ),
    (
        "うまい",
        "この店のラーメンは<b>うまい</b>な。",
        "uncontaminated: everyday spoken, CORE/CASUAL expected",
    ),
    (
        "逼迫",
        "医療体制の<b>逼迫</b>が続いている。",
        "uncontaminated: news vocabulary — does it abstain or reach for COMMON",
    ),
];

#[tokio::main(flavor = "current_thread")]
async fn main() {
    dotenvy::dotenv().ok();
    let api_key =
        std::env::var("JP_TOOLS_ANTHROPIC_API_KEY").expect("set JP_TOOLS_ANTHROPIC_API_KEY");
    let http = reqwest::Client::new();

    let mut tiers = 0;
    for (target, sentence, probing) in PROBES {
        match compactdef::compact_def(&http, &api_key, target, sentence).await {
            Ok(gloss) => {
                let (meaning, tags) = gloss.rsplit_once("<br>").unwrap_or(("", &gloss));
                let has_tier =
                    jp_mine_core::tags::TagLine::parse(tags).is_ok_and(|t| t.familiarity.is_some());
                tiers += usize::from(has_tier);
                println!("{target}\n  {tags}\n  {meaning}\n  probing: {probing}\n");
            }
            Err(e) => println!("{target}\n  FAILED: {e}\n  probing: {probing}\n"),
        }
    }

    // The rate is the point. A rubric that says "omit unless very confident" and
    // still tags everything has not changed anything.
    println!("{tiers}/{} probes carried a familiarity tier", PROBES.len());
}
