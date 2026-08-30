//! What this installation can actually do, probed once and answered in one
//! object.
//!
//! Every optional part of the product is a row in `docs/degradation.md`: what
//! it gives, what happens without it, and the one command that turns it on.
//! This is that table at runtime. The reading surfaces read it to decide which
//! controls to draw, and the doctor prints the `fix` line of everything that is
//! off — so a missing part is a smaller working product with one sentence
//! saying why, never an error.

// Every use of it is in a probe for something only Linux has.
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jp_mine_core::note_type::Imported;
use serde::Serialize;
use serde_json::{Value, json};

use crate::app::AppState;
use crate::error::AppError;

/// Probes run on the reader's first paint, so a slow one would stall the page.
const PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// Long enough that opening both surfaces at once probes once, short enough
/// that starting Anki shows up without a restart.
const CACHE_TTL: Duration = Duration::from_secs(10);

const LAPIS: &str = "Lapis";

#[derive(Serialize, Clone)]
pub struct Capability {
    pub ok: bool,
    /// What is there, in the reader's terms — a count, a name, a path.
    pub detail: String,
    /// The one thing that turns it on. `None` when it is already on.
    pub fix: Option<String>,
    /// Nothing has been read yet and this is why. The dashboard shows the
    /// blocking rows and nothing else, so a first run opens on the one thing that
    /// has to happen rather than on nine empty charts.
    ///
    /// **Only ever set on an install with no lines behind it.** Once there is
    /// history the dashboard has something to say, and gating it would mean
    /// looking at your own statistics required the game to be running — which is
    /// most of the times you would want to.
    ///
    /// Deliberately few: a part is blocking only if the product does not work at
    /// all without it. Anki, audio, a key and the pitch dictionaries are all
    /// things a reader can add later, and asking for them up front is the wall
    /// this exists to remove.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub blocking: bool,
    /// Where in the app the reader fixes this, when the app can do it at all.
    ///
    /// The point of the whole matrix: a `fix` that names a shell command is a
    /// diagnosis, not a repair. `Some` means the surfaces draw a button.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
}

/// A fix the app can perform, as the client needs to know it.
#[derive(Serialize, Clone)]
pub struct Action {
    /// What the button says.
    pub label: String,
    /// Where it goes — a dashboard route, or the overlay's own name for a panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goto: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post: Option<String>,
}

fn on(detail: impl Into<String>) -> Capability {
    Capability {
        ok: true,
        detail: detail.into(),
        fix: None,
        blocking: false,
        action: None,
    }
}

fn off(detail: impl Into<String>, fix: impl Into<String>) -> Capability {
    Capability {
        ok: false,
        detail: detail.into(),
        fix: Some(fix.into()),
        blocking: false,
        action: None,
    }
}

impl Capability {
    /// Nothing reads without it — but say so only while nothing has been read,
    /// which is the only time the reader has nothing else to look at.
    fn blocking(mut self, fresh_install: bool) -> Self {
        self.blocking = fresh_install;
        self
    }

    /// Reachable from inside the app.
    fn fixed_at(mut self, label: &str, goto: &str) -> Self {
        self.action = Some(Action {
            label: label.into(),
            goto: Some(goto.into()),
            post: None,
        });
        self
    }

    fn fixed_by(mut self, label: &str, post: &str) -> Self {
        self.action = Some(Action {
            label: label.into(),
            goto: None,
            post: Some(post.into()),
        });
        self
    }
}

/// Whether a command is on `PATH`. Cheaper than spawning `which`, and this runs
/// on a request path.
#[cfg(unix)]
fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

#[cfg(unix)]
fn first_on_path<'a>(bins: &[&'a str]) -> Option<&'a str> {
    bins.iter().copied().find(|b| on_path(b))
}

#[cfg(unix)]
fn run_dir() -> PathBuf {
    std::env::var_os("VN_RUNDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let base = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", real_uid())));
            base.join("kotodex")
        })
}

/// `getuid` without a libc dependency: the runtime directory is only a fallback
/// path, and `XDG_RUNTIME_DIR` is set in every session that matters.
///
/// `/proc/self/status`, not `/proc/self/loginuid` — that one is the audit login
/// id and reads `4294967295` wherever auditing is off, which is a directory
/// nobody has.
#[cfg(unix)]
fn real_uid() -> u32 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("Uid:"))?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or(1000)
}

/// The ring buffer is live if a segment was written in the last few seconds —
/// ffmpeg rewrites one every 5s, so a stale directory means the daemon is gone
/// even though its files are still there.
#[cfg(unix)]
fn capture_running() -> Capability {
    let seg = run_dir().join("seg");
    let fresh = std::fs::read_dir(&seg)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .any(|m| {
            m.elapsed()
                .map(|e| e < Duration::from_secs(30))
                .unwrap_or(false)
        });
    if fresh {
        on("recording")
    } else {
        off(
            "the ring buffer is not being written",
            "start Kotodex — it runs the capture daemon",
        )
    }
}

/// Where the lines being read come from, and whether the producer is writing.
///
/// The setting is which source was *chosen*; the logger's heartbeat is whether
/// one is actually attached. Both are worth saying: a reader who has picked the
/// clipboard and sees nothing needs to know which of the two is missing.
///
/// Asked of the heartbeat rather than of `lines.log`, which is created on the
/// first run and then outlives every producer — so a file check answers `ok` on
/// a machine with nothing hooked at all.
async fn lines_source(state: &AppState, fresh_install: bool) -> Capability {
    let settings = crate::db::load_settings(&state.local)
        .await
        .unwrap_or_default();
    let beat = super::stream::heartbeat(state).await;
    if beat.as_ref().is_some_and(|b| b.attached()) {
        return on(settings.line_source);
    }
    // Paused is a state the reader chose. The logger disconnects on purpose
    // there, so reporting it as a missing producer would be a fault row for
    // something working exactly as asked.
    if settings.capture_paused && beat.as_ref().is_some_and(|b| b.running()) {
        return on(format!("{}, paused", settings.line_source));
    }
    // Nothing arrives, so there is nothing to read: the one other blocking row.
    // The launcher and the logger it starts are Linux-only, so naming them off
    // Linux would be advice nobody can take. What is true everywhere is that some
    // source has to post to the endpoint.
    #[cfg(not(unix))]
    return match settings.line_source.as_str() {
        "clipboard" => off(
            "clipboard, no producer",
            "nothing is copying text. Start Kotodex, and set the game's \
             clipboard hooker copying.",
        )
        .blocking(fresh_install),
        _ => off(
            "ws, no producer",
            "nothing is hooking the game's text. Set up Textractor with its \
             WebSocket extension, or switch the line source to the clipboard \
             in Settings.",
        )
        .blocking(fresh_install),
    };
    #[cfg(unix)]
    match settings.line_source.as_str() {
        "clipboard" => off(
            "clipboard, no producer",
            "nothing is copying text. Start Kotodex.",
        )
        .blocking(fresh_install),
        _ => off(
            "ws, no producer",
            "nothing is hooking the game's text. Start Kotodex.",
        )
        .blocking(fresh_install),
    }
}

#[cfg(unix)]
fn vad_model() -> Capability {
    let path = std::env::var("VN_VAD_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|_| jp_core::install::data_dir().join("silero_vad.onnx"));
    if path.is_file() {
        on(path.display().to_string())
    } else {
        off("not downloaded", "run the installer again to download it.")
    }
}

#[cfg(unix)]
fn screenshot_tool() -> Capability {
    match first_on_path(&["spectacle", "grim", "gnome-screenshot", "import"]) {
        Some(bin) => on(bin),
        None => off(
            "none installed",
            "install spectacle (KDE), grim (wlroots) or gnome-screenshot",
        ),
    }
}

#[cfg(unix)]
fn xdotool() -> Capability {
    if on_path("xdotool") {
        on("installed")
    } else {
        off(
            "not installed",
            "install xdotool — needed for screenshots and overlay positioning",
        )
    }
}

/// The interpreter the overlay runs under, **asked of `kotodex_python` in
/// `scripts/lib/platform.sh`** rather than worked out again here.
///
/// It has to be the same interpreter the overlay will actually use: which Qt is
/// installed decides whether layer-shell is loadable, so a probe that resolved
/// its own answer would report on a machine nobody is running.
#[cfg(unix)]
fn overlay_python() -> std::path::PathBuf {
    let platform = jp_core::install::install_root().join("scripts/lib/platform.sh");
    let asked = std::process::Command::new("sh")
        .arg("-c")
        // The path as `$1` rather than interpolated into the script, so a
        // directory with a quote or a space in it cannot end the command.
        .arg(". \"$1\" && kotodex_python")
        .arg("sh")
        .arg(&platform)
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .filter(|path| !path.is_empty());
    match asked {
        Some(path) => std::path::PathBuf::from(path),
        // platform.sh unreadable — a broken install rather than a machine to
        // report on. `backend.py` then fails too and the row says so.
        None => std::path::PathBuf::from("python3"),
    }
}

/// Which backend the overlay will pick. Asks `layer-overlay/backend.py` rather
/// than repeating its rules: the choice decides the Qt platform plugin, and two
/// implementations of it would disagree exactly when one of them is wrong.
///
/// Both backends work, so neither answer is a fault — layer-shell is above a
/// fullscreen window by protocol, X11 by `_NET_WM_STATE_ABOVE`.
#[cfg(unix)]
fn overlay_backend() -> Capability {
    let backend_py = jp_core::install::install_root().join("layer-overlay/backend.py");
    let out = std::process::Command::new(overlay_python())
        .arg(&backend_py)
        .output();
    match out {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let (name, why) = text.trim().split_once('\t').unwrap_or((text.trim(), ""));
            if name.is_empty() {
                off("unknown", "layer-overlay/backend.py answered nothing")
            } else if why.is_empty() {
                on(name)
            } else {
                on(format!("{name} — {why}"))
            }
        }
        _ => off("unknown", "could not run layer-overlay/backend.py"),
    }
}

async fn whisper(state: &AppState) -> Capability {
    let url = format!("{}/health", state.whisper_url.trim_end_matches('/'));
    let up = matches!(
        state.http.get(&url).timeout(PROBE_TIMEOUT).send().await,
        Ok(resp) if resp.status().is_success()
    );
    if up {
        on("reachable")
    } else {
        off(
            "not running",
            "whisper-service is not running. Cards still get audio without it, \
             trimmed to the voice instead of the sentence.",
        )
    }
}

/// The Local Audio Server add-on, which is what puts the speaker button in the
/// popup and the word recording on a mined card.
///
/// Asked for a word it certainly has, because the server answers an unknown one
/// with an empty list and a 200 — so a reachable server and a word with no
/// recording look the same from here, and only a word it must know separates
/// them.
async fn local_audio(state: &AppState) -> Capability {
    let sources = crate::services::audio::sources(state, "日本", "にほん").await;
    if sources.is_empty() {
        off(
            "not answering",
            "no word audio. Install the Local Audio Server add-on in Anki and \
             leave Anki running.",
        )
    } else {
        on("reachable")
    }
}

async fn anki(state: &AppState) -> (Capability, Capability) {
    let body = json!({ "action": "modelNames", "version": 6 });
    let resp = state
        .http
        .post(&state.anki_url)
        .timeout(PROBE_TIMEOUT)
        .json(&body)
        .send()
        .await;
    let models: Option<Vec<String>> = match resp {
        Ok(r) => r
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v["result"].clone()).ok()),
        Err(_) => None,
    };
    let Some(models) = models else {
        return (
            off(
                "not running",
                "Anki is not answering. Start it with the AnkiConnect add-on \
                 installed.",
            ),
            off("unknown", "needs a reachable Anki"),
        );
    };
    let want = jp_mine_core::config::AnkiConfig::from_env().model_name;
    let note_type = if models.contains(&want) {
        on(want)
    } else if want == LAPIS {
        off(
            format!("{want} is not in this collection"),
            "required for making cards. Kotodex can download it and import it into Anki.",
        )
        .fixed_by("Import Lapis", "/api/setup/note-type")
    } else {
        off(
            format!("{want} is not in this collection"),
            "required for making cards. Create it in Anki, or set KOTODEX_ANKI_MODEL to a \
             note type you have.",
        )
    };
    (on(format!("{} note types", models.len())), note_type)
}

async fn dictionaries(state: &AppState, fresh_install: bool) -> Value {
    use jp_core::knowledge::dictionaries::{self, Role};
    let all = dictionaries::list_dictionaries(state.knowledge.pool())
        .await
        .unwrap_or_default();
    let count = |role: Role| all.iter().filter(|d| d.role == role).count();
    let definitions = all
        .iter()
        .filter(|d| !matches!(d.role, Role::Frequency | Role::Pitch))
        .count();

    let master = match all.iter().find(|d| d.role == Role::Master) {
        Some(d) => on(d.title.clone()),
        None => off(
            "none",
            "no master dictionary — import a monolingual one (Sankoku is the \
             intended one).",
        ),
    };
    let frequency = match count(Role::Frequency) {
        0 => off(
            "none",
            "no frequency list is imported. Drop one into the dictionaries \
             folder and restart Kotodex.",
        ),
        n => on(format!("{n}")),
    };
    let pitch = match count(Role::Pitch) {
        0 => off(
            "none",
            "no pitch dictionary is imported. Drop one into the dictionaries \
             folder and restart Kotodex.",
        ),
        n => on(format!("{n}")),
    };
    // The one dictionary row that blocks. Without any definitions the popup has
    // nothing to draw, which is most of what reading here is; without a *master*
    // the vocabulary scale has no denominator, which is a figure being absent.
    let defs = match definitions {
        0 => off(
            "none",
            "drop a Yomitan dictionary zip into the dictionaries folder, then \
             restart Kotodex.",
        )
        .blocking(fresh_install),
        n => on(format!("{n}")),
    };
    json!({
        "dict_master": master,
        "dict_definitions": defs,
        "dict_frequency": frequency,
        "dict_pitch": pitch,
    })
}

/// Whether this install has ever read a line.
///
/// The one input deciding whether a missing part *gates* the dashboard: with
/// history behind it there is something to show and nothing to gate. `EXISTS`
/// rather than a count — `lines` is the biggest table here and the question is
/// only whether it has a row.
async fn read_anything(state: &AppState) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM lines)")
        .fetch_one(state.knowledge.pool())
        .await
        .unwrap_or(false)
}

async fn vocabulary_ledger(state: &AppState, read_anything: bool) -> Capability {
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM vocabulary")
        .fetch_one(state.knowledge.pool())
        .await
        .unwrap_or(0);
    if rows > 0 {
        return on(format!("{rows} words"));
    }
    // An install that has read nothing has an empty ledger because there was
    // nothing to fill it with, which is not a fault to report on the run that
    // created it. Empty with lines behind it is one: that is ingest not running.
    if read_anything {
        off(
            "empty",
            "lines have been read but nothing was counted from them",
        )
        .fixed_at("Rebuild the ledger", "#vocab")
    } else {
        on("empty, nothing read yet")
    }
}

async fn explain(state: &AppState) -> Capability {
    if crate::services::llm::available(state).await {
        on("key set")
    } else {
        off(
            "no API key",
            "needed for line explanations and the card gloss",
        )
        .fixed_at("Add a key", "#settings")
    }
}

static CACHE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);

/// The whole matrix. Cached briefly: both reading surfaces ask on open, and
/// several probes are process and filesystem work.
pub async fn probe(state: &AppState) -> Value {
    if let Ok(cache) = CACHE.lock()
        && let Some((at, value)) = cache.as_ref()
        && at.elapsed() < CACHE_TTL
    {
        return value.clone();
    }
    probe_now(state).await
}

/// `GET /api/setup` — the matrix, past the cache.
///
/// What "check again" asks. The reader has just done something outside the app —
/// started Textractor, dropped in a dictionary — and being told to wait ten
/// seconds for the answer to catch up is the worst possible moment for a stale
/// read.
pub async fn setup(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::Json<Value> {
    axum::Json(probe_now(&state).await)
}

pub async fn install_note_type(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<axum::Json<Value>, AppError> {
    let want = jp_mine_core::config::AnkiConfig::from_env().model_name;
    if want != LAPIS {
        return Err(AppError::BadRequest(format!(
            "there is no release to download for {want} — create it in Anki instead"
        )));
    }
    let message = match jp_mine_core::note_type::install_lapis(&state.http, &state.anki_url)
        .await
        .map_err(AppError::Upstream)?
    {
        Imported::Silently => {
            "Lapis is in your collection. It brings its own deck; cards still go to yours.".into()
        }
        Imported::AfterOneClick(path) => format!(
            "Anki's import dialog is open on Lapis — click Import there. \
             The file can be deleted afterwards: {}",
            path.display()
        ),
    };
    Ok(axum::Json(json!({ "message": message })))
}

async fn probe_now(state: &AppState) -> Value {
    // Read once and passed down: it decides which rows gate the dashboard, and
    // two probes asking it separately could answer differently mid-session.
    let read = read_anything(state).await;
    let fresh_install = !read;
    let (anki_up, note_type) = anki(state).await;
    let mut out = json!({
        "lines_source": lines_source(state, fresh_install).await,
        "whisper": whisper(state).await,
        "anki": anki_up,
        "anki_note_type": note_type,
        "local_audio": local_audio(state).await,
        "explain": explain(state).await,
        "vocabulary_ledger": vocabulary_ledger(state, read).await,
    });
    // Capture and the overlay are Linux-only, and a row is a claim that the part
    // could be here. Off with a `fix` naming a package that does not exist for
    // this machine reads as a broken install rather than a smaller one, so the
    // row is absent instead: `kotodex-doctor.sh`'s `cap` skips a key the server
    // did not send, and a surface reading one gets nothing and draws nothing.
    #[cfg(unix)]
    if let Some(out) = out.as_object_mut() {
        out.insert("capture_running".into(), json!(capture_running()));
        out.insert("vad_model".into(), json!(vad_model()));
        out.insert("screenshot_tool".into(), json!(screenshot_tool()));
        out.insert("xdotool".into(), json!(xdotool()));
        out.insert("overlay_backend".into(), json!(overlay_backend()));
    }
    if let (Some(out), Some(dicts)) = (
        out.as_object_mut(),
        dictionaries(state, fresh_install).await.as_object(),
    ) {
        out.extend(dicts.clone());
    }

    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), out.clone()));
    }
    out
}
