//! End-to-end tests over a throwaway database.
//!
//! The unit tests in `src/stats/` pin the derivation rules against synthetic
//! line streams. These pin the layer under them: that the SQL actually selects
//! what the derivations assume, that a paused or discarded line is gone from
//! *every* figure, and that two endpoints looking at the same day agree.

mod support;

use serde_json::json;
use support::{TestApp, now};

/// Start of the current reading day, so fixtures land on "today" whatever the
/// wall clock says. Uses the default rollover hour (4), which is what a fresh
/// test database has.
fn today_start() -> f64 {
    let tz = read_stats::clock::tz_offset_secs();
    let today = read_stats::stats::date_key(now(), 4, tz);
    read_stats::stats::day_start_ts(today, 4, tz)
}

/// Three lines, ten seconds apart, 20 counted characters each. Every gap is
/// inside the default 30s AFK cap, so all of it is credited.
async fn three_lines(app: &TestApp, base: f64, work: Option<&str>) {
    for i in 0..3 {
        app.add_line(
            base + i as f64 * 10.0,
            "あいうえおかきくけこさしすせそたちつてと",
            work,
        )
        .await;
    }
}

#[tokio::test]
async fn summary_counts_todays_lines_and_credits_the_gaps() {
    let app = TestApp::new().await;
    three_lines(&app, today_start() + 3600.0, None).await;

    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["chars"], 60, "20 counted chars × 3 lines");
    assert_eq!(
        s["today"]["active_secs"], 20.0,
        "two 10s gaps, both under the AFK cap"
    );
    assert_eq!(s["today"]["vn"]["chars"], 60);
    assert_eq!(s["today"]["manual"]["chars"], 0);
}

#[tokio::test]
async fn punctuation_does_not_count_as_characters() {
    let app = TestApp::new().await;
    // The texthooker-ui rule: brackets, ellipsis and the comma all drop out.
    app.add_line(today_start() + 3600.0, "「ねえ、聞いてる？」", None)
        .await;

    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["chars"], 6);
}

#[tokio::test]
async fn a_paused_span_removes_its_lines_from_every_figure() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    three_lines(&app, base, None).await;
    // Pause covering the last two lines only.
    sqlx::query("INSERT INTO pauses (start_ts, end_ts) VALUES (?, ?)")
        .bind(base + 5.0)
        .bind(base + 100.0)
        .execute(&app.pool)
        .await
        .unwrap();

    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["chars"], 20, "only the line before the pause");
    assert_eq!(
        s["today"]["active_secs"], 0.0,
        "one surviving line has no gap to credit"
    );
}

#[tokio::test]
async fn discarding_lines_removes_them_and_undo_puts_them_back() {
    let app = TestApp::new().await;
    three_lines(&app, today_start() + 3600.0, None).await;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM lines ORDER BY id")
        .fetch_all(&app.pool)
        .await
        .unwrap();

    let (status, body) = app
        .send(
            "POST",
            "/api/lines/discard",
            json!({ "ids": [ids[0], ids[1]] }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["ids"].as_array().unwrap().len(), 2);

    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["chars"], 20, "one line left");

    let (status, _) = app
        .send(
            "POST",
            "/api/lines/undiscard",
            json!({ "ids": [ids[0], ids[1]] }),
        )
        .await;
    assert_eq!(status, 200);
    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["chars"], 60, "undo restores them");
}

#[tokio::test]
async fn a_manual_session_lands_on_its_day_beside_the_hooked_lines() {
    let app = TestApp::new().await;
    three_lines(&app, today_start() + 3600.0, None).await;

    let (status, session) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "minutes": 30, "chars": 5000, "work": "本" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(session["chars"], 5000);

    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["manual"]["chars"], 5000);
    assert_eq!(s["today"]["manual"]["active_secs"], 1800.0);
    assert_eq!(s["today"]["chars"], 5060, "the total merges both sources");
    // The VN half is untouched by the manual entry.
    assert_eq!(s["today"]["vn"]["chars"], 60);
}

#[tokio::test]
async fn pages_become_characters_when_no_exact_count_is_given() {
    let app = TestApp::new().await;
    let (status, session) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "minutes": 20, "pages": 10 }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(session["chars"], 5500, "10 × the 550 chars/page default");

    let (status, _) = app
        .send("POST", "/api/sessions", json!({ "minutes": 20 }))
        .await;
    assert_eq!(status, 400, "neither chars nor pages");
}

#[tokio::test]
async fn the_day_timeline_and_the_day_total_agree_on_active_time() {
    // The invariant that pace and presence being per-request broke: the
    // dashboard and the timeline priced the same day differently. One session,
    // wholly inside the day, so no boundary effects can excuse a difference.
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    for i in 0..40 {
        // Gaps of 5s and 45s alternating — the long ones exceed the AFK cap, so
        // this exercises the presence rule rather than just summing gaps.
        let ts = base + (i / 2) as f64 * 50.0 + (i % 2) as f64 * 5.0;
        app.add_line(ts, "あいうえおかきくけこ", None).await;
    }
    app.add_lookup(base + 30.0, "何か").await;

    let date = read_stats::stats::date_key(base, 4, read_stats::clock::tz_offset_secs());
    let timeline = app.get(&format!("/api/day/timeline?date={date}")).await;
    let bucket_secs: f64 = timeline["buckets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["active_secs"].as_f64().unwrap())
        .sum();

    let days = app.get("/api/days?days=1").await;
    let day_secs = days[0]["active_secs"].as_f64().unwrap();

    assert!(
        (bucket_secs - day_secs).abs() < 1e-6,
        "timeline {bucket_secs} vs day {day_secs}"
    );
    assert!(day_secs > 0.0, "the fixture must actually credit time");
}

#[tokio::test]
async fn works_lists_read_titles_and_queued_ones_alike() {
    let app = TestApp::new().await;
    three_lines(&app, today_start() + 3600.0, Some("読んでる")).await;
    let (status, _) = app
        .send(
            "POST",
            "/api/works",
            json!({ "title": "積んでる", "status": "queued", "queue_pos": 1 }),
        )
        .await;
    assert_eq!(status, 200);

    let works = app.get("/api/works").await;
    let by_title = |t: &str| {
        works
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["work"] == t)
            .unwrap_or_else(|| panic!("{t} missing from /api/works"))
            .clone()
    };
    assert_eq!(by_title("読んでる")["chars"], 60);
    let queued = by_title("積んでる");
    assert_eq!(queued["chars"], 0, "queued but never read");
    assert_eq!(queued["meta"]["status"], "queued");
    assert!(queued["last_read"].is_null());
}

#[tokio::test]
async fn work_status_must_be_one_of_the_known_values() {
    let app = TestApp::new().await;
    let (status, _) = app
        .send(
            "POST",
            "/api/works",
            json!({ "title": "X", "status": "halfway" }),
        )
        .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn settings_round_trip_and_reject_junk() {
    let app = TestApp::new().await;
    let (status, s) = app
        .send("PUT", "/api/settings", json!({ "afk_secs": 45 }))
        .await;
    assert_eq!(status, 200);
    assert_eq!(s["afk_secs"], 45.0);
    assert_eq!(app.get("/api/settings").await["afk_secs"], 45.0);

    let (status, _) = app
        .send("PUT", "/api/settings", json!({ "nonsense": 1 }))
        .await;
    assert_eq!(status, 400, "unknown key");

    let (status, _) = app
        .send("PUT", "/api/settings", json!({ "afk_secs": "soon" }))
        .await;
    assert_eq!(status, 400, "numeric setting given a string");

    let (status, _) = app
        .send(
            "PUT",
            "/api/settings",
            json!({ "pace_start_date": "yesterday" }),
        )
        .await;
    assert_eq!(status, 400, "unparseable date");
}

#[tokio::test]
async fn changing_the_afk_cap_changes_what_a_long_gap_credits() {
    // Settings are not decoration: the derivation reads them on every request.
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    app.add_line(base, "あいうえお", None).await;
    app.add_line(base + 120.0, "あいうえお", None).await;

    // No pace can be measured (the only gap is over the cap), so the line falls
    // back to the flat cap — which is exactly the setting under test.
    assert_eq!(app.get("/api/summary").await["today"]["active_secs"], 30.0);
    app.send("PUT", "/api/settings", json!({ "afk_secs": 90 }))
        .await;
    assert_eq!(app.get("/api/summary").await["today"]["active_secs"], 90.0);
}

#[tokio::test]
async fn the_mining_funnel_sorts_terms_into_mined_known_and_unmined() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    // A card made *after* the lookup: the lookup led to it.
    app.add_lookup(base, "新出").await;
    // A card made *before* it: a word already carded and still being looked up.
    app.add_lookup(base + 10.0, "手強い").await;
    // Never carded.
    app.add_lookup(base + 20.0, "未収").await;
    app.add_lookup(base + 300.0, "未収").await;

    sqlx::query("INSERT INTO anki_notes (note_id, vocab) VALUES (?, ?), (?, ?)")
        .bind(((base + 60.0) * 1000.0) as i64)
        .bind("新出")
        .bind(((base - 86400.0) * 1000.0) as i64)
        .bind("手強い")
        .execute(&app.pool)
        .await
        .unwrap();

    let s = app.get("/api/lookups/summary").await;
    assert_eq!(s["terms"], 3, "distinct terms, not lookup events");
    assert_eq!(s["events"], 4);
    assert_eq!(s["mined"], 1);
    assert_eq!(s["known"], 1, "the leech");
    assert_eq!(s["unmined"], 1);
    assert_eq!(s["repeat_terms"], 1, "未収 was looked up twice");
    assert_eq!(s["leeches"][0]["term"], "手強い");
    assert_eq!(s["median_mine_secs"], 60.0);
}

#[tokio::test]
async fn dialogue_splits_a_line_by_its_corner_brackets() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    app.add_line(base, "「そうか」と彼は言った", None).await;
    app.add_line(base + 10.0, "あいうえお", None).await;

    let d = app.get("/api/dialogue/summary?days=1").await;
    assert_eq!(d["today"]["dialogue_chars"], 3, "そうか is 3 counted chars");
    assert_eq!(
        d["today"]["narration_chars"],
        6 + 5,
        "と彼は言った + the prose line"
    );
    assert_eq!(d["today"]["share"], 3.0 / 14.0);
}

#[tokio::test]
async fn the_reader_backlog_comes_back_oldest_first() {
    // The feed itself is an SSE stream, but the backlog a client gets on open
    // is this query, and its ordering is the part that has been wrong before:
    // it selects the newest N and must hand them back in reading order.
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    for (i, text) in ["いち", "に", "さん"].iter().enumerate() {
        app.add_line(base + i as f64 * 10.0, text, None).await;
    }

    let lines = read_stats::db::fetch_recent_lines(&app.pool, 2)
        .await
        .unwrap();
    assert_eq!(lines.len(), 2, "capped at the limit");
    assert_eq!(lines[0].text, "に", "the newest two, oldest first");
    assert_eq!(lines[1].text, "さん");
}

#[tokio::test]
async fn reader_state_reports_what_the_phone_can_do() {
    let app = TestApp::new().await;
    let s = app.get("/api/reader/state").await;
    assert_eq!(s["paused"], false);
    assert_eq!(
        s["capture_available"], false,
        "the test app points at no capture script"
    );
    assert_eq!(s["explain_available"], false, "no API key configured");
}

#[tokio::test]
async fn anki_summary_reports_unavailable_before_the_first_refresh() {
    let app = TestApp::new().await;
    app.add_note(1, "何か").await;
    // The snapshot timestamp is what marks a refresh as having happened; notes
    // alone (however they got there) are not enough to report against.
    assert_eq!(app.get("/api/anki/summary").await["available"], false);
}
