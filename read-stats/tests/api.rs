//! End-to-end tests over a throwaway database.
//!
//! The unit tests in `src/stats/` pin the derivation rules against synthetic
//! line streams. These pin the layer under them: that the SQL actually selects
//! what the derivations assume, that a discarded line is gone from *every*
//! figure, and that two endpoints looking at the same day agree.

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
async fn pausing_capture_is_a_flag_and_does_not_touch_the_history() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    three_lines(&app, base, None).await;

    let s = app.get("/api/summary").await;
    assert_eq!(s["paused"], false);
    let chars_before = s["today"]["chars"].clone();

    let (status, body) = app.send("POST", "/api/capture/pause", json!({})).await;
    assert_eq!(status, 200);
    assert_eq!(body["paused"], true);

    // Pausing stops the *logger*, so lines already recorded keep counting —
    // the opposite of the old interval log, which voided them retroactively.
    let s = app.get("/api/summary").await;
    assert_eq!(s["paused"], true);
    assert_eq!(s["today"]["chars"], chars_before, "history is untouched");
    assert_eq!(app.get("/api/reader/state").await["paused"], true);

    let (_, body) = app.send("POST", "/api/capture/pause", json!({})).await;
    assert_eq!(body["paused"], false, "toggles back");
}

#[tokio::test]
async fn retiring_the_pauses_table_discards_the_lines_it_covered() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    three_lines(&app, base, None).await;

    // Recreate the old table exactly as it was, covering the last two lines.
    sqlx::raw_sql(
        "CREATE TABLE pauses (id INTEGER PRIMARY KEY, start_ts REAL NOT NULL, end_ts REAL)",
    )
    .execute(&app.local)
    .await
    .unwrap();
    sqlx::query("INSERT INTO pauses (start_ts, end_ts) VALUES (?, ?)")
        .bind(base + 5.0)
        .bind(base + 100.0)
        .execute(&app.local)
        .await
        .unwrap();

    read_stats::db::retire_pauses(&app.local, &app.knowledge)
        .await
        .unwrap();

    let s = app.get("/api/summary").await;
    assert_eq!(
        s["today"]["chars"], 20,
        "the covered lines are discarded, not merely filtered"
    );
    let discarded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM lines WHERE discarded = 1")
        .fetch_one(app.knowledge.pool())
        .await
        .unwrap();
    assert_eq!(discarded, 2);

    // Second run is a no-op rather than an error: the table is gone.
    read_stats::db::retire_pauses(&app.local, &app.knowledge)
        .await
        .unwrap();
}

#[tokio::test]
async fn discarding_lines_removes_them_and_undo_puts_them_back() {
    let app = TestApp::new().await;
    three_lines(&app, today_start() + 3600.0, None).await;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM lines ORDER BY id")
        .fetch_all(app.knowledge.pool())
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
async fn pasted_text_counts_itself_and_outranks_the_estimate() {
    let app = TestApp::new().await;
    // Eleven counted characters: the 、 and the 。 are punctuation and are not
    // among them, which is the same rule `lines.chars` is held to.
    let content = "本日は快晴なり、風もない。";
    let (status, session) = app
        .send(
            "POST",
            "/api/sessions",
            json!({
                "minutes": 15,
                "pages": 99,
                "content": content,
                "url": "https://example.com/a",
                "work": "記事",
            }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(
        session["chars"], 11,
        "counted from the text, not 99 × chars_per_page"
    );
    assert_eq!(session["source"], "article", "a URL says what it is");
    assert_eq!(session["url"], "https://example.com/a");
    assert_eq!(session["has_content"], true);
    assert!(
        session.get("content").is_none(),
        "the body is never carried on the row"
    );

    let id = session["id"].as_i64().unwrap();
    let fetched = app.get(&format!("/api/sessions/{id}/content")).await;
    assert_eq!(fetched["content"], content);

    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["manual"]["chars"], 11);
}

#[tokio::test]
async fn the_count_endpoint_agrees_with_what_a_session_stores() {
    let app = TestApp::new().await;
    let content = "本日は快晴なり、風もない。";
    let counted = app
        .send("POST", "/api/text/count", json!({ "content": content }))
        .await
        .1;
    let (_, session) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "minutes": 15, "content": content }),
        )
        .await;
    assert_eq!(
        counted["chars"], session["chars"],
        "the form's preview is the stored number"
    );
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
        .execute(app.knowledge.pool())
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

    let lines = read_stats::db::fetch_recent_lines(&app.knowledge, 2)
        .await
        .unwrap();
    assert_eq!(lines.len(), 2, "capped at the limit");
    assert_eq!(lines[0].text, "に", "the newest two, oldest first");
    assert_eq!(lines[1].text, "さん");
}

#[tokio::test]
async fn reader_state_reports_what_the_reader_can_do() {
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

#[tokio::test]
async fn kanji_counts_the_glyphs_and_joins_lookups_and_cards_to_them() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    app.add_line(base, "手強い人と手を組む", None).await;
    app.add_line(base + 10.0, "人がいる", Some("A")).await;
    // Discarded lines must be invisible here as they are everywhere else.
    app.add_line(base + 20.0, "邂逅", None).await;
    let last: i64 = sqlx::query_scalar("SELECT MAX(id) FROM lines")
        .fetch_one(app.knowledge.pool())
        .await
        .unwrap();
    app.send("POST", "/api/lines/discard", json!({ "ids": [last] }))
        .await;

    app.add_lookup(base + 30.0, "手強い").await;
    app.add_note((base * 1000.0) as i64, "手強い").await;

    let s = app.get("/api/kanji").await;
    let rows = s["kanji"].as_array().unwrap();
    let row = |k: &str| {
        rows.iter()
            .find(|r| r["kanji"] == k)
            .unwrap_or_else(|| panic!("{k} missing"))
            .clone()
    };

    assert_eq!(s["total_encounters"], 6, "手×2 強 人×2 組, no 邂逅");
    assert!(!rows.iter().any(|r| r["kanji"] == "邂"), "discarded line");
    assert_eq!(row("人")["count"], 2);
    assert_eq!(row("人")["top_work"], "A", "only one line carried a work");
    assert_eq!(row("手")["lookups"], 1, "手強い contains it");
    assert_eq!(row("手")["mined"], true);
    assert_eq!(row("組")["lookups"], 0);
    assert_eq!(row("組")["mined"], false);
    // The reference table rides along, and 人 is the first kanji taught.
    assert_eq!(row("人")["grade"], 1);
    assert_eq!(row("人")["gloss"], "person");

    let g1 = s["grades"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["grade"] == 1)
        .unwrap()
        .clone();
    assert_eq!(g1["total"], 80);
    assert_eq!(g1["met"], 2, "人 and 手");
    assert_eq!(g1["solid"], 0, "neither reaches the solid threshold");
    assert_eq!(s["days"].as_array().unwrap()[0]["new"], 4);
}

#[tokio::test]
async fn a_lookup_only_counts_when_a_line_arrived_recently() {
    // The rule the AnkiConnect proxy applies before recording a lookup: Yomitan
    // fires for anything looked up in the browser, and only the ones made while
    // a VN was being read belong to the reading. `record` cannot be driven from
    // here without an AnkiConnect to forward to, so this pins the decision it
    // makes — see routes/ankiproxy.rs.
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    app.add_line(base, "あいうえお", Some("読んでる")).await;

    let gap = 600.0; // the default session_gap_secs
    assert!(
        read_stats::db::line_within(&app.knowledge, base + 60.0, gap)
            .await
            .unwrap(),
        "a minute after a line is mid-session"
    );
    assert!(
        read_stats::db::line_within(&app.knowledge, base + gap, gap)
            .await
            .unwrap(),
        "the window is inclusive at its edge"
    );
    assert!(
        !read_stats::db::line_within(&app.knowledge, base + gap + 1.0, gap)
            .await
            .unwrap(),
        "past the session gap the reader is somewhere else"
    );
    assert!(
        !read_stats::db::line_within(&app.knowledge, base - 1.0, gap)
            .await
            .unwrap(),
        "a line that has not happened yet is no evidence"
    );
}

#[tokio::test]
async fn a_discarded_line_is_no_evidence_of_a_session() {
    // Discarding is how a line that should not count is removed everywhere; a
    // lookup must not be admitted by a line the reader has thrown away.
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    app.add_line(base, "あいうえお", None).await;
    sqlx::query("UPDATE lines SET discarded = 1")
        .execute(app.knowledge.pool())
        .await
        .unwrap();

    assert!(
        !read_stats::db::line_within(&app.knowledge, base + 60.0, 600.0)
            .await
            .unwrap()
    );
}
