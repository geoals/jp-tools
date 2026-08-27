//! What this installation can actually do, probed once and answered in one
//! object.
//!
//! Every optional part of the product is a row in `docs/degradation.md`: what
//! it gives, what happens without it, and the one command that turns it on.
//! This is that table at runtime. The reading surfaces read it to decide which
//! controls to draw, and the doctor prints the `fix` line of everything that is
//! off — so a missing part is a smaller working product with one sentence
//! saying why, never an error.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};

use crate::app::AppState;

/// Probes run on the reader's first paint, so a slow one would stall the page.
const PROBE_TIMEOUT: Duration = Duration::from_millis(800);

/// Long enough that opening both surfaces at once probes once, short enough
/// that starting Anki shows up without a restart.
const CACHE_TTL: Duration = Duration::from_secs(10);

#[derive(Serialize, Clone)]
pub struct Capability {
    pub ok: bool,
    /// What is there, in the reader's terms — a count, a name, a path.
    pub detail: String,
    /// The one thing that turns it on. `None` when it is already on.
    pub fix: Option<String>,
}

fn on(detail: impl Into<String>) -> Capability {
    Capability {
        ok: true,
        detail: detail.into(),
        fix: None,
    }
}

fn off(detail: impl Into<String>, fix: impl Into<String>) -> Capability {
    Capability {
        ok: false,
        detail: detail.into(),
        fix: Some(fix.into()),
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
    std::env::var_os("VN_RUNDIR").map(PathBuf::from).unwrap_or_else(|| {
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
        .any(|m| m.elapsed().map(|e| e < Duration::from_secs(30)).unwrap_or(false));
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
async fn lines_source(state: &AppState) -> Capability {
    let settings = crate::db::load_settings(&state.local).await.unwrap_or_default();
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
    // The launcher and the logger it starts are Linux-only, so naming them off
    // Linux would be advice nobody can take. What is true everywhere is that some
    // source has to post to the endpoint.
    #[cfg(not(unix))]
    return off(
        format!("{}, no producer", settings.line_source),
        "no source is posting to /api/lines — see sources/README.md",
    );
    #[cfg(unix)]
    match settings.line_source.as_str() {
        "clipboard" => off(
            "clipboard, no producer",
            "start Kotodex — it runs the capture daemon that watches the clipboard",
        ),
        _ => off(
            "ws, no producer",
            "start Kotodex — it runs the logger that reads Textractor's WebSocket",
        ),
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
        off(
            "not downloaded",
            "run setup.sh again — it downloads the model",
        )
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
        // Continued with a backslash, not a bare line break: a Rust string
        // literal keeps the newline *and* the indentation after it, and this
        // sentence is printed as one line by the doctor and the overlay both.
        off(
            "not installed",
            "install xdotool — required for anki cards to get a screenshot of the \
             right window, and for positioning of the overlay",
        )
    }
}

/// The interpreter the overlay runs under, **asked of `kotodex_python` in
/// `scripts/lib/platform.sh`** rather than worked out again here.
///
/// It has to be the same interpreter the overlay will actually use: which Qt is
/// installed decides whether layer-shell is loadable, so a probe that resolved
/// its own answer would report on a machine nobody is running. That is not a
/// hypothetical — this was a second copy of the rule, and it was the one without
/// the venv check.
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
            "required for trimming card audio to the mined sentence; \
             see whisper-service/README.md",
        )
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
                "start Anki with the AnkiConnect add-on — required for mining",
            ),
            off("unknown", "needs a reachable Anki"),
        );
    };
    let want = jp_mine_core::config::AnkiConfig::from_env().model_name;
    let note_type = if models.contains(&want) {
        on(want)
    } else {
        off(
            format!("{want} is not in this collection"),
            "run the note type check, or set KOTODEX_ANKI_MODEL to one you have",
        )
    };
    (on(format!("{} note types", models.len())), note_type)
}

async fn dictionaries(state: &AppState) -> Value {
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
            "import a monolingual dictionary — required for the vocabulary count",
        ),
    };
    let frequency = match count(Role::Frequency) {
        0 => off(
            "none",
            "import a frequency list — required for underlining common words, \
             the rank in the popup, and review order",
        ),
        n => on(format!("{n}")),
    };
    let pitch = match count(Role::Pitch) {
        0 => off("none", "import a pitch dictionary — required for the accent line"),
        n => on(format!("{n}")),
    };
    let defs = match definitions {
        0 => off(
            "none",
            "drop a Yomitan zip in dictionaries/ and run setup.sh again — \
             required for any definitions",
        ),
        n => on(format!("{n}")),
    };
    json!({
        "dict_master": master,
        "dict_definitions": defs,
        "dict_frequency": frequency,
        "dict_pitch": pitch,
    })
}

async fn vocabulary_ledger(state: &AppState) -> Capability {
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
    //
    // EXISTS rather than a count — `lines` is the biggest table here and the
    // question is only whether it has a row.
    let read_anything: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM lines)")
        .fetch_one(state.knowledge.pool())
        .await
        .unwrap_or(false);
    if read_anything {
        off("empty", "run POST /api/vocab/rebuild — nothing has been ingested")
    } else {
        on("empty, nothing read yet")
    }
}

fn explain(state: &AppState) -> Capability {
    if state.anthropic_api_key.is_some() {
        on("key set")
    } else {
        off(
            "no API key",
            "set KOTODEX_ANTHROPIC_API_KEY — required for AI generated \
             explanation of lines, and word definitions",
        )
    }
}

/// The whole matrix. Cached briefly: both reading surfaces ask on open, and
/// several probes are process and filesystem work.
pub async fn probe(state: &AppState) -> Value {
    static CACHE: Mutex<Option<(Instant, Value)>> = Mutex::new(None);
    if let Ok(cache) = CACHE.lock()
        && let Some((at, value)) = cache.as_ref()
        && at.elapsed() < CACHE_TTL
    {
        return value.clone();
    }

    let (anki_up, note_type) = anki(state).await;
    let mut out = json!({
        "lines_source": lines_source(state).await,
        "whisper": whisper(state).await,
        "anki": anki_up,
        "anki_note_type": note_type,
        "explain": explain(state),
        "vocabulary_ledger": vocabulary_ledger(state).await,
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
    if let (Some(out), Some(dicts)) = (out.as_object_mut(), dictionaries(state).await.as_object()) {
        out.extend(dicts.clone());
    }

    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), out.clone()));
    }
    out
}
