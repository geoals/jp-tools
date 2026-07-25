//! The canonical two-axis tag rubric (FAMILIARITY + FLAVOR) shared by every LLM
//! call that emits it: the CompactDef gloss (`compactdef.rs`) and the reader's
//! explain button (`llm.rs`). Both used to carry their own paraphrase of these
//! definitions and had already drifted apart; this is the single source of truth
//! so a wording change lands everywhere at once. The prose in
//! `spec/anki-compactdef.md` documents the reasoning; keep it in sync with this.
//!
//! FAMILIARITY uses the sharpened definitions: the axis turns on the single
//! question "can you be certain EVERY native adult recognizes it?", with COMMON
//! vs UNCOMMON split by active-vs-passive vocabulary and RARE as the first tier
//! where universal recognition can no longer be assumed.

/// The FAMILIARITY axis — one tier, recognition-on-sight across the population.
pub const FAMILIARITY_RUBRIC: &str = "\
FAMILIARITY (exactly one) — recognition-on-sight across the native adult \
population (NOT frequency, NOT whether they say it). The axis turns on ONE \
question: can you be certain EVERY native adult recognizes it?\n\
- CORE — every native, from childhood.\n\
- COMMON — every native adult knows it, and for most it is ACTIVE vocabulary \
(they would use it themselves).\n\
- UNCOMMON — essentially every native adult still RECOGNIZES it, but for a large \
portion it is PASSIVE only (known, but they would not produce it).\n\
- RARE — the first tier where you CANNOT be certain every adult knows it. Many \
do, but a large share of such words are recognized mainly by people who read.\n\
- OBSCURE — you can assume non-readers do NOT know it, and even among active \
readers only a portion recognize it.\n\
A transparent compound of common parts with a predictable meaning (等価値 = \
等価+価値) is understood first-encounter → COMMON or higher. Spoken/colloquial \
words are more familiar than their rarity in writing suggests; don't demote them \
for being informal.";

/// The FLAVOR axis — one baseline formality plus up to two independent marks.
pub const FLAVOR_RUBRIC: &str = "\
FLAVOR (1-3) — if you SAY it in the wrong room, how do you sound. Emit exactly \
one baseline formality, then add marks only when they carry an independent, \
equally-important warning:\n\
- baseline: SLANG / PLAIN (safe anywhere — always shown) / FORMAL (stiff if \
casual; fine in formal speech or writing) / LITERARY (writing-only; theatrical \
if spoken).\n\
- marks: TECHNICAL, RELIGIOUS, HONORIFIC, HUMBLE, DIALECT, ARCHAIC, VULGAR, \
DEROGATORY, CHILDISH.\n\
Tag the IN-SENTENCE sense; other senses don't count (joking 成仏 = PLAIN, not \
RELIGIOUS). A word can be marked in origin but plain in use — tag current usage, \
not etymology.";
