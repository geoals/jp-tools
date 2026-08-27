//! The one report this program makes: a mined card is complete.
//!
//! Enrichment is deliberately invisible — the card exists the moment Yomitan
//! adds it, and the media and the definition arrive seconds later, in a
//! background task, in a browser tab that is not the one being read. That is
//! the right shape for the reading, but it leaves nothing at all to tell you
//! the card actually came out whole, which matters precisely because both
//! halves can fail quietly (a stale ring buffer, a note open in Anki's editor).
//!
//! Sent once, at the end, only when nothing failed.

use tracing::debug;

/// Notification timeout in milliseconds. Long enough to catch out of the corner
/// of the eye while reading, short enough not to sit on the game.
const TIMEOUT_MS: u32 = 2000;

/// Report the finished card, if reporting is on.
///
/// Detached and never awaited: a mine is finished whether or not a notification
/// came out of it, and blocking the enrichment task on a notification daemon
/// would be a strange way to find that out. No notification daemon, or no
/// session bus, is silence logged at debug — this is feedback, not a feature
/// anything depends on.
///
/// Set `KOTODEX_MINE_NOTIFY=0` to mine in silence.
pub fn mine_complete(word: &str) {
    if std::env::var("KOTODEX_MINE_NOTIFY").is_ok_and(|v| v == "0") {
        return;
    }
    let body = if word.is_empty() {
        "Card complete".to_string()
    } else {
        format!("{word} — card complete")
    };
    // Blocking: the session bus call is synchronous, so it goes on the blocking
    // pool rather than holding an enrichment task's thread.
    tokio::task::spawn_blocking(move || {
        // The product's name and its desktop-entry id: the first is what the
        // notification is labelled with, the second is what a notification
        // daemon looks the icon up under.
        if let Err(e) = notify_rust::Notification::new()
            .appname("Kotodex")
            .icon("com.kotodex.Kotodex")
            .summary("✅ Mined")
            .body(&body)
            .timeout(notify_rust::Timeout::Milliseconds(TIMEOUT_MS))
            .show()
        {
            debug!(error = %e, "could not send the mine notification");
        }
    });
}
