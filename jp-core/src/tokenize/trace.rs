//! Why the tokenizer produced the tokens it did.
//!
//! The pipeline is a sequence of small decisions — a rewrite of the line, a
//! wordhood gate per morpheme, two split passes, a stammer filter, an identity
//! ladder per token, and a recomposition pass that offers runs of tokens up to
//! be joined. Each of those is already an isolated predicate in the parent
//! module; this is the channel they announce themselves on.
//!
//! **Recording is the same code path as tokenizing, never a second one.** An
//! explanation derived by a parallel implementation would drift from the thing
//! it explains. So [`Trace`] is threaded through the real functions and is
//! inert when off — [`Trace::push`] takes a closure, so a step nobody asked for
//! is a bool check and never a `format!`.

/// One decision, in the order it was made.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Step {
    /// The pipeline's one edit to its input: emphatic っ removed. Absent when
    /// the line had none.
    Rewrite { from: String, to: String },
    /// Sudachi's own segmentation, before any of our rules run. Every later
    /// step is about these pieces, so a boundary that was never ours to make
    /// has to be visible as such — otherwise the trace reads as if the
    /// pipeline chose to start there.
    Segment {
        mode: &'static str,
        parts: Vec<String>,
    },
    /// The wordhood gate: is this compound a word in its own right, so the
    /// C→B→A pass should stop here rather than take it apart?
    Gate {
        surface: String,
        kept: bool,
        why: String,
    },
    /// A morpheme the gate refused, handed to a finer mode.
    Split {
        surface: String,
        mode: &'static str,
        parts: Vec<String>,
    },
    /// A stammer's leading fragment, dropped.
    Stutter { fragment: String, into: String },
    /// Which `(headword, reading)` a token was filed under, and which rung of
    /// the ladder settled it.
    Identity {
        surface: String,
        headword: String,
        reading: String,
        rule: &'static str,
        /// The ladder as it stood after filtering, most canonical first.
        candidates: Vec<String>,
    },
    /// A run of adjacent tokens offered to recomposition.
    Join {
        parts: Vec<String>,
        #[serde(flatten)]
        verdict: Verdict,
    },
}

/// What became of one candidate run.
///
/// [`NoSignal`](Verdict::NoSignal) is split out from a refusal because
/// recomposition offers every run of two and three tokens at every position and
/// nearly all of them spell nothing. A reader looking for why a compound came
/// apart wants the runs that named a word and were turned down anyway.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    Joined {
        term: String,
        reading: String,
        signal: &'static str,
    },
    /// A signal named a word, and a later rule refused it.
    Refused { term: String, reason: String },
    /// Neither the spelling nor the reading named a listed word.
    NoSignal { reason: &'static str },
}

/// The recorder. Off by default and inert when off; see the module doc.
#[derive(Debug, Default)]
pub struct Trace {
    on: bool,
    steps: Vec<Step>,
}

impl Trace {
    pub fn off() -> Self {
        Self::default()
    }

    pub fn recording() -> Self {
        Self {
            on: true,
            steps: Vec::new(),
        }
    }

    pub fn is_recording(&self) -> bool {
        self.on
    }

    /// Record a step. The closure runs only while recording, so building the
    /// step costs nothing on the hot path.
    pub fn push(&mut self, step: impl FnOnce() -> Step) {
        if self.on {
            self.steps.push(step());
        }
    }

    pub fn into_steps(self) -> Vec<Step> {
        self.steps
    }
}
