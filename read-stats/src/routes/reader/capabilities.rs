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
fn on_path(bin: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
        .unwrap_or(false)
}

fn first_on_path<'a>(bins: &[&'a str]) -> Option<&'a str> {
    bins.iter().copied().find(|b| on_path(b))
}

fn run_dir() -> PathBuf {
    std::env::var_os("VN_RUNDIR").map(PathBuf::from).unwrap_or_else(|| {
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc_getuid() })));
        base.join("vn-mine")
    })
}

/// `getuid` without a libc dependency: the runtime directory is only a fallback
/// path, and `XDG_RUNTIME_DIR` is set in every session that matters.
fn libc_getuid() -> u32 {
    std::fs::read_to_string("/proc/self/loginuid")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1000)
}

/// The ring buffer is live if a segment was written in the last few seconds —
/// ffmpeg rewrites one every 5s, so a stale directory means the daemon is gone
/// even though its files are still there.
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
            "start it: vn-mine/vn-buffer.sh run",
        )
    }
}

/// Where the lines being read come from, and whether the producer is writing.
///
/// The setting is which source was *chosen*; `lines.log` is whether one is
/// actually running. Both are worth saying: a reader who has picked the
/// clipboard and sees nothing needs to know which of the two is missing.
async fn lines_source(state: &AppState) -> Capability {
    let chosen = crate::db::load_settings(&state.local)
        .await
        .map(|s| s.line_source)
        .unwrap_or_else(|_| "ws".into());
    if run_dir().join("lines.log").is_file() {
        return on(chosen);
    }
    match chosen.as_str() {
        "clipboard" => off(
            "clipboard, no producer",
            "start the capture daemon — it is what watches the clipboard",
        ),
        _ => off(
            "ws, no producer",
            "run Textractor with its WebSocket plugin pointed at vn-ws-logger.py",
        ),
    }
}

fn vad_model() -> Capability {
    let path = std::env::var("VN_VAD_MODEL").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
            .join(".local/share/vn-mine/silero_vad.onnx")
    });
    if path.is_file() {
        on(path.display().to_string())
    } else {
        off(
            "not downloaded",
            "run setup.sh, or fetch silero_vad.onnx into ~/.local/share/vn-mine/",
        )
    }
}

fn screenshot_tool() -> Capability {
    match first_on_path(&["spectacle", "grim", "gnome-screenshot", "import"]) {
        Some(bin) => on(bin),
        None => off(
            "none installed",
            "install spectacle (KDE), grim (wlroots) or gnome-screenshot",
        ),
    }
}

fn xdotool() -> Capability {
    if on_path("xdotool") {
        on("installed")
    } else {
        off(
            "not installed",
            "install xdotool — without it the screenshot takes whatever has focus",
        )
    }
}

/// Which backend the overlay will pick. Asks `layer-overlay/backend.py` rather
/// than repeating its rules: the choice decides the Qt platform plugin, and two
/// implementations of it would disagree exactly when one of them is wrong.
///
/// Both backends work, so neither answer is a fault — layer-shell is above a
/// fullscreen window by protocol, X11 by `_NET_WM_STATE_ABOVE`.
fn overlay_backend() -> Capability {
    let backend_py = jp_core::install::install_root().join("layer-overlay/backend.py");
    let out = std::process::Command::new("python3").arg(&backend_py).output();
    match out {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout);
            let (name, why) = text.trim().split_once('\t').unwrap_or((text.trim(), ""));
            if name.is_empty() {
                off("unknown", "layer-overlay/backend.py answered nothing")
            } else if why.is_empty() {
                on(name)
            } else {
                on(&format!("{name} — {why}"))
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
            "optional — it narrows the clip to the mined sentence; see whisper-service/README.md",
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
                "start Anki with the AnkiConnect add-on — mining is off until then",
            ),
            off("unknown", "needs a reachable Anki"),
        );
    };
    let want = jp_mine_core::config::AnkiConfig::from_env().model_name;
    let note_type = if models.iter().any(|m| *m == want) {
        on(want)
    } else {
        off(
            format!("{want} is not in this collection"),
            "run the note type check, or set JP_TOOLS_ANKI_MODEL to one you have",
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
            "import a monolingual dictionary — without one there is no vocabulary scale",
        ),
    };
    let frequency = match count(Role::Frequency) {
        0 => off(
            "none",
            "import a frequency list — no underline, no rank pill, no ordering by how common a word is",
        ),
        n => on(format!("{n}")),
    };
    let pitch = match count(Role::Pitch) {
        0 => off("none", "import a pitch dictionary to show the accent"),
        n => on(format!("{n}")),
    };
    let defs = match definitions {
        0 => off(
            "none",
            "drop a Yomitan zip in dictionaries/ and run jp-dict sync — the popup is empty without one",
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
        on(format!("{rows} words"))
    } else {
        off(
            "empty",
            "it fills itself as you read; status marks stay off until it has rows",
        )
    }
}

fn explain(state: &AppState) -> Capability {
    if state.anthropic_api_key.is_some() {
        on("key set")
    } else {
        off(
            "no API key",
            "set JP_TOOLS_ANTHROPIC_API_KEY to explain a line",
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
        "capture_running": capture_running(),
        "lines_source": lines_source(state).await,
        "vad_model": vad_model(),
        "screenshot_tool": screenshot_tool(),
        "xdotool": xdotool(),
        "overlay_backend": overlay_backend(),
        "whisper": whisper(state).await,
        "anki": anki_up,
        "anki_note_type": note_type,
        "explain": explain(state),
        "vocabulary_ledger": vocabulary_ledger(state).await,
    });
    if let (Some(out), Some(dicts)) = (out.as_object_mut(), dictionaries(state).await.as_object()) {
        out.extend(dicts.clone());
    }

    if let Ok(mut cache) = CACHE.lock() {
        *cache = Some((Instant::now(), out.clone()));
    }
    out
}
