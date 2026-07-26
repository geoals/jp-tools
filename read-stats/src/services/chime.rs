//! The one sound this program makes: a mined card is complete.
//!
//! Enrichment is deliberately invisible — the card exists the moment Yomitan
//! adds it, and the media and the definition arrive seconds later, in a
//! background task, in a browser tab that is not the one being read. That is
//! the right shape for the reading, but it leaves nothing at all to tell you
//! the card actually came out whole, which matters precisely because both
//! halves can fail quietly (a stale ring buffer, a note open in Anki's editor).
//!
//! `import` used to ring the X bell as a side effect of every screenshot, which
//! Plasma turned into an audible beep; `-silent` stopped it, and that turned out
//! to have been doing real work. This is the same feedback made deliberate:
//! played once, at the end, only when nothing failed.

use std::path::PathBuf;
use std::sync::LazyLock;

use tracing::debug;

/// The sound file, overridable with `JP_TOOLS_MINE_CHIME`. Set it to an empty
/// value to mine in silence.
static CHIME: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let path = std::env::var("JP_TOOLS_MINE_CHIME")
        .unwrap_or_else(|_| "/usr/share/sounds/freedesktop/stereo/complete.oga".to_string());
    (!path.is_empty()).then(|| PathBuf::from(path))
});

/// Play the completion sound, if there is one to play.
///
/// Detached and never awaited: a mine is finished whether or not a sound came
/// out of it, and blocking the enrichment task on an audio server would be a
/// strange way to find that out. A missing player or a missing file is silence,
/// logged at debug — this is feedback, not a feature anything depends on.
pub fn mine_complete() {
    let Some(path) = CHIME.as_ref() else {
        return;
    };
    if !path.is_file() {
        debug!(path = %path.display(), "mine chime file not found");
        return;
    }
    match tokio::process::Command::new("paplay")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        // Waited on in a task of its own rather than dropped on the floor: the
        // caller does not block, and the child is still reaped instead of
        // accumulating one zombie per mined card across a reading session.
        Ok(mut child) => {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        }
        Err(e) => debug!(error = %e, "could not play the mine chime"),
    }
}
