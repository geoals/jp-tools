//! The reading view (`#read`): a live feed of the lines Textractor hooks, plus
//! the buttons that act on them.
//!
//! Read in a browser beside the running VN, so Yomitan can scan the lines
//! without reaching into the game window. Yomitan's AnkiConnect endpoint is
//! `/anki-proxy`, which forwards to the Anki on the machine running the VN —
//! the same collection vn-capture.sh attaches media to. The view is served over
//! the LAN as well, so a second device beside the screen works the same way.
//!
//! Lines are read out of the `lines` table rather than from Textractor's
//! WebSocket directly: vn-ws-logger.py is already writing them there, and its
//! WS plugin can crash Textractor when a client disconnects abortively, so a
//! second WS client is a risk with nothing to gain.
//!
//! | module | does |
//! |---|---|
//! | [`stream`] | the SSE line feed, resumable by `Last-Event-ID` |
//! | [`highlight`] | which words in a line are worth marking, and where |
//! | [`lines`] | clearing lines from the figures, and undoing that |
//! | [`capture`] | the mine button, and the window picker behind it |
//! | [`define`] | what a word means, for the overlay's popup |
//! | [`audio`] | what a word sounds like, from the audio server beside Anki |
//! | [`mine`] | making a card from the overlay, as Yomitan makes one |
//! | [`mined`] | whether a word is already a card, and the way to it |
//! | [`explain`] | "what does this line say", via the model |
//! | [`state`] | what the page can do, in one round trip on open |

pub mod audio;
pub mod capture;
pub mod define;
pub mod explain;
pub mod highlight;
pub mod lines;
pub mod mine;
pub mod mined;
pub mod state;
pub mod stream;

use tracing::warn;

use crate::app::AppState;
use crate::clock::now_ts;
use crate::db;

/// Record that the reader did something deliberate on this page just now, so
/// [`crate::stats::Presence`] credits the surrounding gap even when no Yomitan
/// lookup or mined card landed in it (reading an explanation, mining without a
/// fresh lookup). Best-effort: a failed insert is logged, never propagated —
/// the action it accompanies must still succeed.
pub(crate) async fn mark_presence(state: &AppState, kind: &str) {
    if let Err(e) = db::insert_reader_mark(&state.local, now_ts(), kind).await {
        warn!(error = %e, kind, "failed to record reader presence mark");
    }
}
