//! A read-stats instance backed by a throwaway SQLite file.
//!
//! The endpoints are driven through the real router rather than by calling
//! handlers directly, so the tests cover the routing, the extractors and the
//! JSON shape the SPA actually consumes — everything except the socket.

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use read_stats::app::{AppState, build_router};
use read_stats::db;
use serde_json::Value;
use sqlx::SqlitePool;
use tower::ServiceExt;

pub struct TestApp {
    pub pool: SqlitePool,
    pub router: Router,
    db_path: PathBuf,
}

impl TestApp {
    pub async fn new() -> Self {
        // Unique per test: the suite runs threads in parallel and SQLite files
        // are process-wide state.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("read-stats-test-{nanos}.db"));
        let pool = db::create_pool(db_path.to_str().unwrap()).await.unwrap();

        let state = AppState {
            pool: pool.clone(),
            covers_dir: std::env::temp_dir().join("read-stats-test-covers"),
            http: reqwest::Client::new(),
            // Pointed at the discard port: no test may reach a real service.
            anki_url: "http://127.0.0.1:9".into(),
            anki_deck: "Japanese".into(),
            anki_vocab_field: "VocabKanji".into(),
            anki_sentence_field: "SentKanji".into(),
            anki_compact_def_field: "CompactDef".into(),
            auto_capture_on_add: false,
            sudachi_dict_path: PathBuf::from("system_full.dic"),
            vn_capture_script: PathBuf::from("/nonexistent"),
            anthropic_api_key: None,
            whisper_url: "http://127.0.0.1:9".into(),
        };
        TestApp {
            router: build_router(state),
            pool,
            db_path,
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

    /// Append a hooked line the way vn-ws-logger.py does. `chars` is left to
    /// the startup recount in the real pipeline, so it is computed here too.
    pub async fn add_line(&self, ts: f64, text: &str, work: Option<&str>) {
        sqlx::query("INSERT INTO lines (ts, chars, text, work) VALUES (?, ?, ?, ?)")
            .bind(ts)
            .bind(read_stats::charcount::count_chars(text))
            .bind(text)
            .bind(work)
            .execute(&self.pool)
            .await
            .unwrap();
    }

    pub async fn add_lookup(&self, ts: f64, term: &str) {
        // dedupe window 0 so fixtures get exactly the rows they ask for.
        db::insert_lookup(&self.pool, ts, term, None, 0.0)
            .await
            .unwrap();
    }

    pub async fn add_note(&self, note_id: i64, vocab: &str) {
        db::replace_anki_notes(
            &self.pool,
            &[db::AnkiNote {
                note_id,
                vocab: vocab.into(),
            }],
        )
        .await
        .unwrap();
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", self.db_path.display()));
        }
    }
}

/// Now, rounded down to a whole second — fixtures place lines relative to this
/// so they land on today whatever the rollover hour is.
pub fn now() -> f64 {
    read_stats::clock::now_ts().floor()
}
