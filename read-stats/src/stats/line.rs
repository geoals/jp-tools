//! The single input type every derivation in [`crate::stats`] consumes.
//!
//! A hooked line reduced to the two things any aggregate needs: when it was
//! read and how much text it carried. Everything else about the row (its id,
//! its work, the text itself) stays in the database.

#[derive(Debug, Clone, Copy, Default)]
pub struct LineEvent {
    pub ts: f64,
    pub chars: i64,
}
