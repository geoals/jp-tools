//! The single input type every derivation in [`crate::stats`] consumes.
//!
//! A hooked line reduced to the four things any aggregate needs: when it was
//! read, how much text it carried, and how that text splits between speech and
//! prose. Everything else about the row (its id, its work, the text itself)
//! stays in the database — derivation never needs it.

use crate::dialogue;

#[derive(Debug, Clone, Copy, Default)]
pub struct LineEvent {
    pub ts: f64,
    pub chars: i64,
    /// How many of `chars` sat inside 「」/『』 — see [`crate::dialogue`].
    /// Narration is the remainder, so this stays a partition of `chars` and
    /// nothing downstream has to reconcile two counts. Zero for rows stored
    /// without text, which read as pure narration; the dialogue aggregates
    /// exclude those rather than believe them.
    pub dialogue_chars: i64,
    /// Whether the row had text to classify at all.
    pub classified: bool,
}

impl LineEvent {
    pub fn narration_chars(&self) -> i64 {
        self.chars - self.dialogue_chars
    }

    /// Which side of the split the whole line falls on, or `None` when it is
    /// mixed or unclassifiable.
    pub fn kind(&self) -> Option<dialogue::Kind> {
        self.classified.then(|| {
            dialogue::Split {
                dialogue: self.dialogue_chars,
                narration: self.narration_chars(),
            }
            .kind()
        })?
    }
}
