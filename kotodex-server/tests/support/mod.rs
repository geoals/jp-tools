//! A kotodex-server instance backed by a throwaway SQLite file.
//!
//! The endpoints are driven through the real router rather than by calling
//! handlers directly, so the tests cover the routing, the extractors and the
//! JSON shape the SPA actually consumes — everything except the socket.

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use jp_core::knowledge::Knowledge;
use kotodex_server::app::{AppState, build_router};
use kotodex_server::db;
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;

pub struct TestApp {
    /// kotodex-server's own database.
    pub local: SqlitePool,
    /// The shared knowledge database — where the line stream lives.
    pub knowledge: Knowledge,
    pub router: Router,
    /// For tests that drive a background pass (ingest) rather than an endpoint.
    pub state: AppState,
    paths: Vec<PathBuf>,
}

impl TestApp {
    pub async fn new() -> Self {
        // Unique per test: the suite runs threads in parallel and SQLite files
        // are process-wide state.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("kotodex-server-test-{nanos}.db"));
        let knowledge_path =
            std::env::temp_dir().join(format!("kotodex-server-test-knowledge-{nanos}.db"));
        let local = db::create_pool(db_path.to_str().unwrap()).await.unwrap();
        let knowledge = db::open_knowledge(knowledge_path.to_str().unwrap(), true)
            .await
            .unwrap();

        let state = AppState {
            local: local.clone(),
            knowledge: knowledge.clone(),
            covers_dir: std::env::temp_dir().join("kotodex-server-test-covers"),
            http: reqwest::Client::new(),
            // Pointed at the discard port: no test may reach a real service.
            anki_url: "http://127.0.0.1:9".into(),
            anki: Default::default(),
            anki_deck: "Japanese".into(),
            anki_vocab_field: "VocabKanji".into(),
            anki_sentence_field: "SentKanji".into(),
            anki_compact_def_field: "CompactDef".into(),
            auto_capture_on_add: false,
            // Absolute, from the workspace root: cargo runs tests with the
            // *package* as the working directory, so the config default's
            // relative path would not resolve here.
            sudachi_dict_path: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../system_full.dic"),
            vn_capture_script: PathBuf::from("/nonexistent"),
            anthropic_api_key: None,
            whisper_url: "http://127.0.0.1:9".into(),
            // Port 9 is discard: nothing answers, which is the audio server
            // being absent — the case the popup has to survive.
            local_audio_url: "http://127.0.0.1:9".into(),
            highlighter: Default::default(),
            demo: false,
        };
        TestApp {
            router: build_router(state.clone()),
            state,
            local,
            knowledge,
            paths: vec![db_path, knowledge_path],
        }
    }

    pub async fn get(&self, uri: &str) -> Value {
        let res = self
            .router
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(
            res.status().is_success(),
            "GET {uri} returned {}",
            res.status()
        );
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Send a JSON body and return `(status, parsed body)` — the error cases
    /// matter as much as the happy path for the settings endpoint.
    pub async fn send(&self, method: &str, uri: &str, body: Value) -> (u16, Value) {
        let res = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, value)
    }

    /// POST a raw body — the epub upload is the file itself, not a form.
    pub async fn send_bytes(&self, uri: &str, body: Vec<u8>) -> (u16, Value) {
        let res = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = res.status().as_u16();
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// Append a hooked line the way vn-ws-logger.py does. `chars` is left to
    /// the startup recount in the real pipeline, so it is computed here too.
    pub async fn add_line(&self, ts: f64, text: &str, work: Option<&str>) {
        sqlx::query("INSERT INTO lines (ts, chars, text, work) VALUES (?, ?, ?, ?)")
            .bind(ts)
            .bind(jp_core::text::chars::count_chars(text))
            .bind(text)
            .bind(work)
            .execute(self.knowledge.pool())
            .await
            .unwrap();
    }

    pub async fn add_lookup(&self, ts: f64, term: &str) {
        // dedupe window 0 so fixtures get exactly the rows they ask for.
        db::insert_lookup(&self.knowledge, ts, term, None, 0.0)
            .await
            .unwrap();
    }

    pub async fn add_note(&self, note_id: i64, vocab: &str) {
        self.add_notes(&[(note_id, vocab)]).await;
    }

    /// The whole deck snapshot in one call.
    ///
    /// `replace_anki_notes` mirrors the deck wholesale, so calling `add_note`
    /// twice leaves only the second — a fixture that needs several notes has
    /// to set them together, exactly as a real refresh does.
    pub async fn add_notes(&self, notes: &[(i64, &str)]) {
        let notes: Vec<db::AnkiNote> = notes
            .iter()
            // No tokenizer in a fixture, so a card is spelt the way the ledger
            // keys it — the case where normalizing changes nothing.
            .map(|(note_id, vocab)| db::AnkiNote {
                note_id: *note_id,
                vocab: (*vocab).into(),
                headword: (*vocab).into(),
            })
            .collect();
        db::replace_anki_notes(&self.knowledge, &notes)
            .await
            .unwrap();
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        for path in &self.paths {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
            }
        }
    }
}

/// Now, rounded down to a whole second — fixtures place lines relative to this
/// so they land on today whatever the rollover hour is.
pub fn now() -> f64 {
    kotodex_server::clock::now_ts().floor()
}
