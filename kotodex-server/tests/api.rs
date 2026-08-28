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
    let tz = kotodex_server::clock::tz_offset_secs();
    let today = kotodex_server::stats::date_key(now(), 4, tz);
    kotodex_server::stats::day_start_ts(today, 4, tz)
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

    kotodex_server::db::retire_pauses(&app.local, &app.knowledge)
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
    kotodex_server::db::retire_pauses(&app.local, &app.knowledge)
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
async fn an_untimed_session_gets_its_minutes_from_the_readers_own_pace() {
    let app = TestApp::new().await;
    // Enough hooked reading to establish an effective pace: 800 lines, 20
    // counted chars each, 5s apart — 16000 chars over ~4000s credited, which
    // clears the one-hour floor and works out at ~4 chars/sec.
    let base = today_start() - 3600.0 * 4.0;
    for i in 0..800 {
        app.add_line(
            base + i as f64 * 5.0,
            "あいうえおかきくけこさしすせそたちつてと",
            None,
        )
        .await;
    }

    let (status, session) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "chars": 8000, "work": "時間を計っていない本" }),
        )
        .await;
    assert_eq!(status, 200, "minutes are not required");
    assert!(
        session["end_ts"].is_null(),
        "nothing measured, so nothing stored"
    );

    // 8000 chars at ~4 chars/sec is ~2000s. The assertion is a band, not a
    // point: what must hold is that the estimate comes from the measured pace
    // at all, not that the fixture's arithmetic is reproduced here.
    let listed = app.get("/api/sessions").await;
    let manual = &listed["manual"][0];
    assert_eq!(manual["estimated"], true);
    let secs = manual["active_secs"].as_f64().unwrap();
    assert!(
        (1500.0..2500.0).contains(&secs),
        "derived from ~8000 chars at the measured pace, got {secs}s"
    );
    assert_eq!(
        manual["end_ts"].as_f64().unwrap(),
        manual["start_ts"].as_f64().unwrap() + secs,
        "the span the client draws is the resolved one"
    );

    // And it reaches the day total, which is what the goal meter reads.
    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["manual"]["active_secs"].as_f64().unwrap(), secs);
}

#[tokio::test]
async fn a_timed_session_keeps_the_time_it_was_given() {
    let app = TestApp::new().await;
    let (_, session) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "minutes": 45, "chars": 8000 }),
        )
        .await;
    assert_eq!(
        session["end_ts"].as_f64().unwrap() - session["start_ts"].as_f64().unwrap(),
        2700.0
    );

    let manual = &app.get("/api/sessions").await["manual"][0];
    assert_eq!(manual["estimated"], false, "measured, not derived");
    assert_eq!(manual["active_secs"], 2700.0);

    let (status, _) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "minutes": 0, "chars": 10 }),
        )
        .await;
    assert_eq!(status, 400, "given, it still has to be a real duration");
}

#[tokio::test]
async fn an_untimed_session_with_no_pace_behind_it_claims_no_time() {
    // A fresh database has no reading to measure a pace from. The session is
    // still logged — its characters are real — but it must not invent minutes
    // that would land in the streak.
    let app = TestApp::new().await;
    let (status, _) = app
        .send("POST", "/api/sessions", json!({ "chars": 8000 }))
        .await;
    assert_eq!(status, 200);

    let manual = &app.get("/api/sessions").await["manual"][0];
    assert_eq!(manual["estimated"], true);
    assert_eq!(manual["active_secs"], 0.0);
    assert_eq!(manual["chars"], 8000, "the characters still count");
}

#[tokio::test]
async fn todays_date_anchors_at_now_not_at_mid_day() {
    // The form pre-fills today, so this is the ordinary path — and a session
    // logged in the evening must not land in the morning of the day timeline.
    let app = TestApp::new().await;
    let tz = kotodex_server::clock::tz_offset_secs();
    let today = kotodex_server::stats::date_key(now(), 4, tz);

    let (status, session) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "date": today.to_string(), "minutes": 30, "chars": 5000 }),
        )
        .await;
    assert_eq!(status, 200);
    let end = session["end_ts"].as_f64().unwrap();
    assert!(
        (end - now()).abs() < 60.0,
        "the reading ends about now, not at mid-day"
    );

    // A date genuinely in the past still anchors at mid-day: there is no
    // better guess for when in that day it happened.
    let then = today - chrono::Duration::days(9);
    let (_, old) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "date": then.to_string(), "minutes": 30, "chars": 5000 }),
        )
        .await;
    let expect = kotodex_server::stats::day_start_ts(then, 4, tz) + 8.0 * 3600.0;
    assert_eq!(old["start_ts"].as_f64().unwrap(), expect);
}

#[tokio::test]
async fn a_date_ahead_of_the_reading_day_does_not_land_in_the_future() {
    // Between midnight and the 04:00 rollover the browser's calendar date is
    // already tomorrow while the reading day is still today, so the pre-filled
    // date is one ahead. Its mid-day anchor would be hours from now.
    let app = TestApp::new().await;
    let tz = kotodex_server::clock::tz_offset_secs();
    let tomorrow = kotodex_server::stats::date_key(now(), 4, tz) + chrono::Duration::days(1);

    let (status, session) = app
        .send(
            "POST",
            "/api/sessions",
            json!({ "date": tomorrow.to_string(), "chars": 5000 }),
        )
        .await;
    assert_eq!(status, 200);
    assert!(
        session["start_ts"].as_f64().unwrap() <= now() + 1.0,
        "reading cannot have happened later than now"
    );
}

/// Ignored by default: loads the 360 MB Sudachi dictionary, which is far more
/// than the rest of the suite costs put together.
#[tokio::test]
#[ignore = "requires system_full.dic"]
async fn a_sessions_pasted_text_becomes_word_days() {
    let app = TestApp::new().await;
    app.send(
        "POST",
        "/api/sessions",
        json!({ "content": "政府は予算案を決定した。予算案の説明が必要だ。" }),
    )
    .await;

    let out = kotodex_server::ingest::ingest_new_sessions(&app.state)
        .await
        .unwrap();
    assert_eq!(out.lines, 1, "one session tokenized");
    assert!(out.words > 0);

    let counts: Vec<(String, i64)> =
        sqlx::query_as("SELECT lemma, count FROM word_days ORDER BY count DESC, lemma")
            .fetch_all(app.knowledge.pool())
            .await
            .unwrap();
    let by_lemma: std::collections::HashMap<_, _> = counts.into_iter().collect();
    // Mode C keeps 予算案 whole rather than splitting it into 予算 + 案 — the
    // same compound handling the line stream gets, which is what makes a
    // pasted article's counts comparable with a hooked one's.
    assert_eq!(
        by_lemma.get("予算案"),
        Some(&2),
        "counted once per occurrence, across both sentences"
    );
    assert!(by_lemma.contains_key("政府"));
    // Particles and copulas are not content words and never enter the ledger.
    assert!(!by_lemma.contains_key("は"));

    // The watermark makes a second run a no-op rather than a double count.
    let again = kotodex_server::ingest::ingest_new_sessions(&app.state)
        .await
        .unwrap();
    assert_eq!(again.lines, 0);
    let total: i64 = sqlx::query_scalar("SELECT sum(count) FROM word_days")
        .fetch_one(app.knowledge.pool())
        .await
        .unwrap();
    assert_eq!(
        by_lemma.values().sum::<i64>(),
        total,
        "nothing counted twice"
    );
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

    let date = kotodex_server::stats::date_key(base, 4, kotodex_server::clock::tz_offset_secs());
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
async fn works_lists_read_titles_and_planned_ones_alike() {
    let app = TestApp::new().await;
    three_lines(&app, today_start() + 3600.0, Some("読んでる")).await;
    let (status, _) = app
        .send(
            "POST",
            "/api/works",
            json!({ "title": "積んでる", "status": "planned", "queue_pos": 1 }),
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
    let planned = by_title("積んでる");
    assert_eq!(planned["chars"], 0, "planned but never read");
    assert_eq!(planned["meta"]["status"], "planned");
    assert!(planned["last_read"].is_null());
    // The kind filter feeds on this: a title the texthooker stamped is a VN,
    // and so is one merely queued.
    assert_eq!(by_title("読んでる")["kind"], "vn");
    assert_eq!(planned["kind"], "vn");
}

#[tokio::test]
async fn works_derive_kind_from_what_wrote_them() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    three_lines(&app, base, Some("VN")).await;
    // A book has no line stream — it enters by the log form, which says so.
    app.send(
        "POST",
        "/api/sessions",
        json!({ "start_ts": base - 86400.0, "minutes": 30, "chars": 5000, "work": "本" }),
    )
    .await;
    // An article collapses into the synthetic Articles work.
    app.send(
        "POST",
        "/api/sessions",
        json!({ "start_ts": base - 2.0 * 86400.0, "minutes": 10, "chars": 2000, "source": "article", "url": "https://example.com/x" }),
    )
    .await;

    let works = app.get("/api/works").await;
    let by_title = |t: &str| {
        works
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["work"] == t)
            .unwrap_or_else(|| panic!("{t} missing from /api/works"))
    };
    assert_eq!(by_title("VN")["kind"], "vn");
    assert_eq!(by_title("本")["kind"], "book");
    assert_eq!(by_title("Articles")["kind"], "articles");
}

#[tokio::test]
async fn work_detail_scopes_the_history_to_one_title() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    three_lines(&app, base, Some("A")).await;
    // Another VN, hours later: a sitting of its own that must not appear in A's.
    three_lines(&app, base + 30000.0, Some("B")).await;
    app.add_note(((base + 10.0) * 1000.0) as i64, "言葉").await;
    app.send(
        "POST",
        "/api/works",
        json!({ "title": "A", "total_chars": 200 }),
    )
    .await;

    let d = app.get("/api/works/detail?work=A").await;
    assert_eq!(d["chars"], 60, "only A's lines");
    assert_eq!(d["meta"]["total_chars"], 200);
    assert_eq!(d["remaining_chars"], 140);
    assert_eq!(d["days"].as_array().unwrap().len(), 1);
    let sittings = d["sittings"].as_array().unwrap();
    assert_eq!(sittings.len(), 1, "B's sitting belongs to B");
    assert_eq!(sittings[0]["chars"], 60);
    assert_eq!(sittings[0]["cards"], 1, "the card was mined during it");
    assert_eq!(sittings[0]["estimated"], false);

    // A logged session joins by the same title, and reports its time as an
    // estimate when it was logged without minutes.
    app.send(
        "POST",
        "/api/sessions",
        json!({ "start_ts": base - 86400.0, "chars": 500, "source": "vn", "work": "A" }),
    )
    .await;
    let d = app.get("/api/works/detail?work=A").await;
    assert_eq!(d["chars"], 560);
    assert_eq!(d["days"].as_array().unwrap().len(), 2);
    let sittings = d["sittings"].as_array().unwrap();
    assert_eq!(sittings.len(), 2);
    assert!(
        sittings[0]["start_ts"].as_f64().unwrap() > sittings[1]["start_ts"].as_f64().unwrap(),
        "newest first"
    );
    assert_eq!(sittings[1]["estimated"], true, "logged without minutes");
}

/// A card belongs to whatever was on screen when it was added, and to nothing
/// at all when nothing was.
#[tokio::test]
async fn cards_attribute_to_the_work_that_was_being_read() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    three_lines(&app, base, Some("A")).await;
    three_lines(&app, base + 30000.0, Some("B")).await;

    // Mid-sitting in A, seconds after A's last line, mid-sitting in B, and one
    // in the small hours with no reading anywhere near it. One call: the
    // snapshot is replaced wholesale, never appended to.
    app.add_notes(&[
        (((base + 15.0) * 1000.0) as i64, "近い"),
        (((base + 30.0) * 1000.0) as i64, "直後"),
        (((base + 30010.0) * 1000.0) as i64, "別作"),
        (((base + 90000.0) * 1000.0) as i64, "無関係"),
    ])
    .await;

    let a = app.get("/api/works/detail?work=A").await;
    assert_eq!(
        a["mined_count"], 2,
        "both cards added while A was on screen"
    );
    let vocab: Vec<&str> = a["mined"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["vocab"].as_str().unwrap())
        .collect();
    assert!(vocab.contains(&"近い") && vocab.contains(&"直後"));

    let b = app.get("/api/works/detail?work=B").await;
    assert_eq!(b["mined_count"], 1);
    assert_eq!(b["mined"][0]["vocab"], "別作");
    // The unattributed one belongs to neither rather than to the nearer.
    assert_eq!(
        a["mined_count"].as_i64().unwrap() + b["mined_count"].as_i64().unwrap(),
        3,
        "a card added while nothing was hooked is claimed by no work"
    );

    // And it lands on the right day of the right work's history.
    let day = a["days"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["cards"].as_i64().unwrap() > 0)
        .expect("a day with cards");
    assert_eq!(day["cards"], 2);
}

#[tokio::test]
async fn work_detail_needs_a_work_that_exists() {
    let app = TestApp::new().await;
    let (status, _) = app
        .send("GET", "/api/works/detail?work=nothing", json!({}))
        .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn work_status_must_be_one_of_the_known_values() {
    let app = TestApp::new().await;
    for bad in ["halfway", "queued"] {
        let (status, _) = app
            .send("POST", "/api/works", json!({ "title": "X", "status": bad }))
            .await;
        assert_eq!(status, 400, "{bad} is not a status");
    }
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
async fn the_vocabulary_scale_counts_master_terms_not_ledger_rows() {
    let app = TestApp::new().await;
    // Two known words, but only one of them is vocabulary: the other is a
    // phrase Jitendex happens to give a headword to. The scale must not count
    // it — that is the whole reason `in_master` exists.
    sqlx::query(
        "INSERT INTO vocabulary (headword, reading, status, in_master) VALUES \
             ('読む', 'よむ', 'known', 1), ('ああ見えても', '', 'known', 0), \
             ('憂鬱', 'ゆううつ', 'new', 1), ('五月蝿い', 'うるさい', 'unknown', 1)",
    )
    .execute(app.knowledge.pool())
    .await
    .unwrap();

    let v = app.get("/api/vocab/summary").await;
    assert_eq!(v["total"], 4, "every row, whatever its status");
    assert_eq!(v["known_in_master"], 1, "known *and* vocabulary");

    let known = v["by_status"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["status"] == "known")
        .unwrap()
        .clone();
    assert_eq!(known["total"], 2);
    assert_eq!(known["in_master"], 1);
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

    let lines = kotodex_server::db::fetch_recent_lines(&app.knowledge, 2)
        .await
        .unwrap();
    assert_eq!(lines.len(), 2, "capped at the limit");
    assert_eq!(lines[0].text, "に", "the newest two, oldest first");
    assert_eq!(lines[1].text, "さん");
}

#[tokio::test]
async fn the_reader_opens_on_the_whole_sitting_not_a_fixed_tail() {
    // What a fresh client gets on open. The boundary must be the one
    // `derive_sessions` splits on, so the header the feed draws over these
    // lines names the same sitting the dashboard would.
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    // An earlier sitting, then a gap well over the 600s default, then the
    // current one — deliberately more lines than the old 40-line backlog was
    // never the point; the point is that the split lands between the two.
    app.add_line(base, "むかし", None).await;
    app.add_line(base + 10.0, "むかしむかし", None).await;
    app.add_line(base + 5000.0, "いち", None).await;
    app.add_line(base + 5010.0, "に", None).await;
    app.add_line(base + 5020.0, "さん", None).await;

    let lines = kotodex_server::db::fetch_current_session_lines(&app.knowledge, 600.0, 500)
        .await
        .unwrap();
    let texts: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        texts,
        ["いち", "に", "さん"],
        "only the sitting in progress, oldest first — the 5000s gap ends it"
    );

    // A gap wide enough to swallow the break makes it all one sitting, which
    // is what proves the cut is the gap rule and not a fixed count.
    let all = kotodex_server::db::fetch_current_session_lines(&app.knowledge, 6000.0, 500)
        .await
        .unwrap();
    assert_eq!(all.len(), 5, "one sitting under a wider gap");

    // And the cap still bounds a marathon, taking the newest of it.
    let capped = kotodex_server::db::fetch_current_session_lines(&app.knowledge, 6000.0, 2)
        .await
        .unwrap();
    assert_eq!(
        capped.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["に", "さん"],
        "capped to the newest, still oldest first"
    );
}

#[tokio::test]
async fn scrolling_back_pages_through_history_without_gaps_or_overlap() {
    // The backscroll endpoint. Chained by feeding each page's oldest id back
    // as `before`, it must tile the history exactly once.
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    for i in 0..10 {
        app.add_line(base + i as f64 * 10.0, "あ", None).await;
    }

    let newest = kotodex_server::db::max_line_id(&app.knowledge)
        .await
        .unwrap();
    let mut before = newest + 1;
    let mut seen: Vec<i64> = Vec::new();
    loop {
        let page = kotodex_server::db::fetch_lines_before_id(&app.knowledge, before, 4)
            .await
            .unwrap();
        if page.is_empty() {
            break;
        }
        let ids: Vec<i64> = page.iter().map(|l| l.id).collect();
        assert!(ids.windows(2).all(|w| w[0] < w[1]), "page is oldest-first");
        assert!(
            ids.iter().all(|&i| i < before),
            "a page must precede `before`"
        );
        before = ids[0];
        // Pages arrive newest-first, so each one goes on the front.
        seen.splice(0..0, ids);
    }
    assert_eq!(seen.len(), 10, "every line, once");
    assert!(
        seen.windows(2).all(|w| w[0] < w[1]),
        "no overlap and no reordering across pages"
    );
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
    // The explain button is always drawn; without a key it opens the field to
    // paste one into. What it costs is a capability row, not a hidden control.
    assert_eq!(s["capabilities"]["explain"]["ok"], false);
    assert!(
        s["capabilities"]["explain"]["fix"].is_string(),
        "an off capability always says what turns it on"
    );
}

/// The gate decides whether the dashboard is replaced by the setup page, so it
/// is the one capability field worth asserting exactly: a `blocking` row that
/// survives once there is history would mean looking at your own statistics
/// required the game to be running.
#[tokio::test]
async fn parts_block_the_dashboard_only_until_something_has_been_read() {
    let app = TestApp::new().await;

    let fresh = app.get("/api/setup").await;
    let blocking: Vec<String> = blocking_rows(&fresh);
    assert!(
        blocking.contains(&"lines_source".to_string()),
        "a fresh install has nothing hooked and nothing to show: {blocking:?}"
    );
    assert!(
        blocking.contains(&"dict_definitions".to_string()),
        "a fresh install has no dictionary: {blocking:?}"
    );
    for row in &blocking {
        assert!(
            fresh[row]["fix"].is_string(),
            "{row} blocks the page and must say what to do"
        );
    }

    app.add_line(1_700_000_000.0, "何か読んだ", Some("作品"))
        .await;

    let read = app.get("/api/setup").await;
    assert_eq!(
        blocking_rows(&read),
        Vec::<String>::new(),
        "with history behind it nothing gates the dashboard, however much is off"
    );
    assert_eq!(
        read["lines_source"]["ok"], false,
        "the row is still off — it just no longer takes the page over"
    );
}

fn blocking_rows(caps: &serde_json::Value) -> Vec<String> {
    let mut rows: Vec<String> = caps
        .as_object()
        .expect("the matrix is an object")
        .iter()
        .filter(|(_, c)| c["blocking"] == true)
        .map(|(k, _)| k.clone())
        .collect();
    rows.sort();
    rows
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
        kotodex_server::db::line_within(&app.knowledge, base + 60.0, gap)
            .await
            .unwrap(),
        "a minute after a line is mid-session"
    );
    assert!(
        kotodex_server::db::line_within(&app.knowledge, base + gap, gap)
            .await
            .unwrap(),
        "the window is inclusive at its edge"
    );
    assert!(
        !kotodex_server::db::line_within(&app.knowledge, base + gap + 1.0, gap)
            .await
            .unwrap(),
        "past the session gap the reader is somewhere else"
    );
    assert!(
        !kotodex_server::db::line_within(&app.knowledge, base - 1.0, gap)
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
        !kotodex_server::db::line_within(&app.knowledge, base + 60.0, 600.0)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn the_triage_queue_ticks_only_words_never_looked_up() {
    let app = TestApp::new().await;
    // Three unjudged master-dictionary words, differing only in what the
    // reader's history says about them.
    sqlx::query(
        "INSERT INTO vocabulary (headword, reading, status, in_master, encounter_count, lookup_count) VALUES \
             ('憂鬱', 'ゆううつ', 'new', 1, 47, 0), \
             ('齟齬', 'そご', 'new', 1, 31, 4), \
             ('逼迫', 'ひっぱく', 'new', 1, 1, 0), \
             ('っっ', '', 'new', 0, 99, 0)",
    )
    .execute(app.knowledge.pool())
    .await
    .unwrap();

    let q = app.get("/api/vocab/queue").await;
    assert_eq!(q["min_encounters"], 3, "the default setting");
    let terms = q["terms"].as_array().unwrap();
    assert_eq!(
        terms.len(),
        2,
        "逼迫 is under the floor, っっ is not vocabulary"
    );

    let by = |w: &str| {
        terms
            .iter()
            .find(|t| t["headword"] == w)
            .unwrap_or_else(|| panic!("{w} missing from the queue"))
            .clone()
    };
    assert_eq!(by("憂鬱")["preselect"], true, "met often, never looked up");
    assert_eq!(by("齟齬")["preselect"], false, "looked up 4 times");

    assert_eq!(q["pending"], 2);
    assert_eq!(q["pending_preselected"], 1);

    // The threshold is previewable per request, so the UI can show what
    // lowering it would pull in before anything is saved.
    let low = app.get("/api/vocab/queue?min_encounters=1").await;
    assert_eq!(low["pending"], 3, "逼迫 joins at a floor of 1");
}

#[tokio::test]
async fn triage_writes_both_verdicts_and_leaves_the_rest_new() {
    let app = TestApp::new().await;
    sqlx::query(
        "INSERT INTO vocabulary (headword, reading, status, in_master, encounter_count) VALUES \
             ('憂鬱', 'ゆううつ', 'new', 1, 9), ('齟齬', 'そご', 'new', 1, 9), \
             ('逼迫', 'ひっぱく', 'new', 1, 9)",
    )
    .execute(app.knowledge.pool())
    .await
    .unwrap();

    let (code, body) = app
        .send(
            "POST",
            "/api/vocab/judge",
            serde_json::json!({ "judgements": [
                { "headword": "憂鬱", "reading": "ゆううつ", "status": "known" },
                { "headword": "齟齬", "reading": "そご", "status": "unknown" },
            ]}),
        )
        .await;
    assert_eq!(code, 200);
    assert_eq!(body["written"], 2);

    // 逼迫 was in the batch's page but not in the batch: submitting a sweep
    // must not judge a word the reader never answered for.
    let v = app.get("/api/vocab/summary").await;
    let count = |s: &str| {
        v["by_status"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["status"] == s)
            .map(|c| c["total"].as_i64().unwrap())
            .unwrap_or(0)
    };
    assert_eq!(count("known"), 1);
    assert_eq!(count("unknown"), 1);
    assert_eq!(count("new"), 1, "逼迫 is untouched");

    // A judged word leaves the queue, which is what stops triage re-asking.
    let q = app.get("/api/vocab/queue").await;
    assert_eq!(q["pending"], 1);
}

/// The periodic sweep: a batch scoped to what has
/// been read since the last one, a watermark that moves on submit and not on
/// load, and a backlog that is always still reachable.
#[tokio::test]
async fn a_sweep_scopes_to_what_was_read_since_the_last_one() {
    let app = TestApp::new().await;
    let now = now();
    sqlx::query(
        "INSERT INTO vocabulary (headword, reading, status, in_master, encounter_count, last_seen) VALUES \
             ('憂鬱', 'ゆううつ', 'new', 1, 9, ?), ('齟齬', 'そご', 'new', 1, 9, ?)",
    )
    .bind(now - 86_400.0)
    .bind(now - 86_400.0)
    .execute(app.knowledge.pool())
    .await
    .unwrap();

    // Never swept: the first batch is the whole backlog, not an empty list.
    let q = app.get("/api/vocab/queue").await;
    assert_eq!(q["scoped"], false, "no watermark yet");
    assert_eq!(q["pending"], 2);
    assert_eq!(app.get("/api/vocab/summary").await["ready_since"], 2);

    // Judging without `advance_sweep` must not retire a batch: the watermark
    // belongs to the sweep, not to any write that happens to pass through.
    let (code, body) = app
        .send(
            "POST",
            "/api/vocab/judge",
            json!({ "judgements": [
                { "headword": "憂鬱", "reading": "ゆううつ", "status": "known" },
            ]}),
        )
        .await;
    assert_eq!(code, 200);
    assert_eq!(body["swept_through"], serde_json::Value::Null);
    assert_eq!(app.get("/api/vocab/queue").await["scoped"], false);

    // The sweep's own submit moves it.
    let (code, body) = app
        .send(
            "POST",
            "/api/vocab/judge",
            json!({ "advance_sweep": true, "judgements": [
                { "headword": "齟齬", "reading": "そご", "status": "unknown" },
            ]}),
        )
        .await;
    assert_eq!(code, 200);
    assert!(body["swept_through"].as_f64().unwrap() >= now);

    // A word met only before the mark is not in the next batch, and a word met
    // since it is.
    sqlx::query(
        "INSERT INTO vocabulary (headword, reading, status, in_master, encounter_count, last_seen) VALUES \
             ('逼迫', 'ひっぱく', 'new', 1, 9, ?), ('曖昧', 'あいまい', 'new', 1, 9, ?)",
    )
    .bind(now - 86_400.0)
    .bind(now + 60.0)
    .execute(app.knowledge.pool())
    .await
    .unwrap();

    let q = app.get("/api/vocab/queue").await;
    assert_eq!(q["scoped"], true);
    let words: Vec<&str> = q["terms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["headword"].as_str().unwrap())
        .collect();
    assert_eq!(words, vec!["曖昧"], "逼迫 was last read before the mark");
    assert_eq!(q["pending"], 1);

    // The tile agrees with the batch, and the backlog is still one flag away.
    let v = app.get("/api/vocab/summary").await;
    assert_eq!(v["ready_since"], 1);
    assert_eq!(v["ready"], 2, "逼迫 is still ready, just not new");
    assert_eq!(app.get("/api/vocab/queue?scoped=0").await["pending"], 2);
}

#[tokio::test]
async fn an_unknown_status_is_refused_rather_than_defaulted() {
    let app = TestApp::new().await;
    // Status::parse falls back to `new`; if the endpoint used it, a typo would
    // silently un-judge a word while reporting success.
    let (code, _) = app
        .send(
            "POST",
            "/api/vocab/judge",
            serde_json::json!({ "judgements": [
                { "headword": "憂鬱", "reading": "ゆううつ", "status": "knwon" },
            ]}),
        )
        .await;
    assert_eq!(code, 400);

    let v = app.get("/api/vocab/summary").await;
    assert_eq!(v["total"], 0, "nothing was written");
}

#[tokio::test]
async fn bulk_blacklist_clears_noise_but_not_words() {
    let app = TestApp::new().await;
    sqlx::query(
        "INSERT INTO vocabulary (headword, reading, status, in_master, in_name, in_reference, encounter_count) VALUES \
             ('あああ', '', 'new', 0, 0, 0, 14), \
             ('憂鬱', 'ゆううつ', 'new', 1, 0, 0, 9), \
             ('ああ見えても', '', 'new', 0, 0, 1, 3), \
             ('岡部', 'おかべ', 'new', 0, 1, 0, 31)",
    )
    .execute(app.knowledge.pool())
    .await
    .unwrap();

    let (code, body) = app
        .send(
            "POST",
            "/api/vocab/blacklist-non-words",
            serde_json::json!({}),
        )
        .await;
    assert_eq!(code, 200);
    assert_eq!(
        body["blacklisted"], 1,
        "only あああ — a Jitendex phrase and a name are still words"
    );
}

#[tokio::test]
async fn a_lookup_credits_the_ledger_key_not_the_spelling_it_was_made_under() {
    // Yomitan sends the word as the text spelt it, and the ledger keys on
    // Sudachi's normalized form. Joining those as raw strings credited 検死 —
    // a row nothing else writes to — while 検屍, the row the reader actually
    // meets, read as never looked up. `preselects_known` ticks a word `known`
    // on encounters alone when `lookup_count` is 0, so the cost of the miss was
    // a wrong assertion written in bulk.
    //
    // The normalization itself needs a Sudachi dictionary; what is pinned here
    // is that `sync_lookup_counts` follows `headword` once it is resolved.
    use jp_core::knowledge::vocabulary;

    let app = TestApp::new().await;
    let ts = today_start() + 3600.0;

    for (headword, reading) in [("検屍", "けんし"), ("検死", "けんし")] {
        vocabulary::record_encounters(
            &app.knowledge,
            &[vocabulary::Encounter {
                term: vocabulary::Term::new(headword.to_string(), reading),
                pos: None,
                count: 3,
                first_ts: ts,
                last_ts: ts,
            }],
        )
        .await
        .unwrap();
    }

    kotodex_server::db::insert_lookup(&app.knowledge, ts, "検死", None, 0.0)
        .await
        .unwrap();
    kotodex_server::db::set_lookup_headwords(
        &app.knowledge,
        &[("検死".to_string(), "検屍".to_string())],
    )
    .await
    .unwrap();
    vocabulary::sync_lookup_counts(&app.knowledge)
        .await
        .unwrap();

    async fn lookup_count(k: &jp_core::knowledge::Knowledge, headword: &str) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT lookup_count FROM vocabulary WHERE headword = ?")
            .bind(headword)
            .fetch_one(k.pool())
            .await
            .unwrap()
    }
    assert_eq!(
        lookup_count(&app.knowledge, "検屍").await,
        1,
        "the word the reader met is credited"
    );
    assert_eq!(
        lookup_count(&app.knowledge, "検死").await,
        0,
        "the spelling Yomitan sent is not a second word"
    );
}

/// Marking a word known from the mobile popup takes back the lookup that
/// opening the popup recorded — the popup was the only way to reach the button.
///
/// Deleted, not flagged: every figure is derived from these rows at query time,
/// so the retracted lookup has to be absent rather than marked.
#[tokio::test]
async fn a_retracted_lookup_is_gone_from_the_table() {
    let app = TestApp::new().await;
    let id = kotodex_server::db::insert_lookup(&app.knowledge, 1000.0, "夥しい", None, 0.0)
        .await
        .unwrap()
        .expect("a fresh lookup is its own row");

    let (status, body) = app
        .send(
            "POST",
            "/api/reader/lookup/retract",
            serde_json::json!({ "lookup_id": id, "term": "違う" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["retracted"], false, "the term guards the id");
    assert_eq!(rows(&app).await, 1);

    let (status, body) = app
        .send(
            "POST",
            "/api/reader/lookup/retract",
            serde_json::json!({ "lookup_id": id, "term": "夥しい" }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["retracted"], true);
    assert_eq!(rows(&app).await, 0);
    // A lookup is presence evidence as well as a lookup. Taking the row away
    // must not take away the proof that the reader was at the screen.
    assert_eq!(
        kotodex_server::db::fetch_reader_marks(&app.local, 0.0, f64::MAX)
            .await
            .unwrap(),
        vec![1000.0],
        "the deleted lookup leaves its presence mark, at its own instant"
    );

    async fn rows(app: &TestApp) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lookups")
            .fetch_one(app.knowledge.pool())
            .await
            .unwrap()
    }
}

/// A second popup on the same word inside the dedupe window writes no row, and
/// must still name the row it is accounted to — otherwise that popup's known
/// mark has nothing to retract and the lookup survives.
#[tokio::test]
async fn a_deduped_lookup_names_the_row_it_folded_into() {
    let app = TestApp::new().await;
    let first = kotodex_server::db::insert_lookup(&app.knowledge, 1000.0, "所謂", None, 60.0)
        .await
        .unwrap();
    let second = kotodex_server::db::insert_lookup(&app.knowledge, 1010.0, "所謂", None, 60.0)
        .await
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM lookups")
            .fetch_one(app.knowledge.pool())
            .await
            .unwrap(),
        1
    );
}

// ── Books on paper ────────────────────────────────────────────────────────
//
// The seam worth pinning is the whole round trip: an epub goes in, an anchor
// typed off the page comes back as an offset, and what is logged is an
// ordinary manual session carrying the text between two positions.

/// A two-document epub whose body starts after a page of front matter.
fn tiny_epub() -> Vec<u8> {
    use std::io::Write;
    let files = [
        (
            "META-INF/container.xml",
            r#"<container><rootfiles><rootfile full-path="OEBPS/book.opf"/></rootfiles></container>"#
                .to_string(),
        ),
        (
            "OEBPS/book.opf",
            r#"<package><manifest>
                 <item id="front" href="front.xhtml"/>
                 <item id="one" href="one.xhtml"/>
               </manifest><spine>
                 <itemref idref="front"/><itemref idref="one"/>
               </spine></package>"#
                .to_string(),
        ),
        (
            "OEBPS/front.xhtml",
            "<html><body><p>この本は縦書きです。</p></body></html>".to_string(),
        ),
        (
            "OEBPS/one.xhtml",
            "<html><body><p>少女は<ruby>言<rt>い</rt></ruby>った。</p>\
             <p>それきり黙った。</p><p>雨が降っていた。</p></body></html>"
                .to_string(),
        ),
    ];
    let mut buf = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
        for (name, body) in files {
            zip.start_file::<_, ()>(name, Default::default()).unwrap();
            zip.write_all(body.as_bytes()).unwrap();
        }
        zip.finish().unwrap();
    }
    buf
}

#[tokio::test]
async fn a_book_is_logged_from_the_line_it_was_left_on() {
    let app = TestApp::new().await;
    let (status, up) = app
        .send_bytes(
            "/api/books/upload?title=%E3%83%86%E3%82%B9%E3%83%88",
            tiny_epub(),
        )
        .await;
    assert_eq!(status, 200, "{up}");
    // The ruby reading is not text: 言った, never 言いった.
    assert!(up["head"].as_str().unwrap().contains("少女は言った。"));

    let (status, setup) = app
        .send(
            "POST",
            "/api/books/setup",
            json!({ "work": "テスト", "anchor": "少女は言った", "first_page": 1, "last_page": 2 }),
        )
        .await;
    assert_eq!(status, 200, "{setup}");
    // The front matter is before the anchor and is not part of the book.
    assert_eq!(
        setup["body_chars"], 20,
        "少女は言った それきり黙った 雨が降っていた"
    );

    let (status, preview) = app
        .send(
            "POST",
            "/api/books/preview",
            json!({ "work": "テスト", "anchor": "それきり黙った", "minutes": 6 }),
        )
        .await;
    assert_eq!(status, 200, "{preview}");
    assert_eq!(preview["chars"], 13, "both sentences, punctuation excluded");
    assert_eq!(preview["speed"], 130.0, "13 chars in 6 minutes");
    // The 。 was not typed, so it is not part of the match — and does not count.
    assert_eq!(preview["found"]["after"], "。\n雨が降っていた。");

    let end = preview["found"]["end"].clone();
    let (status, logged) = app
        .send(
            "POST",
            "/api/books/log",
            json!({ "work": "テスト", "end": end, "minutes": 6 }),
        )
        .await;
    assert_eq!(status, 200, "{logged}");
    assert_eq!(logged["session"]["chars"], 13);
    assert_eq!(logged["session"]["source"], "book");
    assert_eq!(logged["session"]["work"], "テスト");
    // 20 characters over two printed pages, so 13 of them is 1.3 pages.
    assert!((logged["session"]["pages"].as_f64().unwrap() - 1.3).abs() < 1e-9);

    // The position moved, and the same anchor cannot be logged twice.
    let books = app.get("/api/books").await;
    assert_eq!(books["books"][0]["position"], end);
    let (status, err) = app
        .send(
            "POST",
            "/api/books/log",
            json!({ "work": "テスト", "end": end }),
        )
        .await;
    assert_eq!(status, 400, "{err}");
}

#[tokio::test]
async fn the_anchor_search_only_ever_looks_forward() {
    let app = TestApp::new().await;
    app.send_bytes(
        "/api/books/upload?title=%E3%83%86%E3%82%B9%E3%83%88",
        tiny_epub(),
    )
    .await;
    app.send(
        "POST",
        "/api/books/setup",
        json!({ "work": "テスト", "anchor": "少女は言った" }),
    )
    .await;
    let (_, preview) = app
        .send(
            "POST",
            "/api/books/preview",
            json!({ "work": "テスト", "anchor": "それきり黙った" }),
        )
        .await;
    let end = preview["found"]["end"].clone();
    app.send(
        "POST",
        "/api/books/log",
        json!({ "work": "テスト", "end": end }),
    )
    .await;

    // Already read, so it is behind the position and no longer findable.
    let (status, err) = app
        .send(
            "POST",
            "/api/books/preview",
            json!({ "work": "テスト", "anchor": "少女は言った" }),
        )
        .await;
    assert_eq!(status, 400, "{err}");
}

#[tokio::test]
async fn a_book_already_part_read_is_caught_up_without_logging_it() {
    let app = TestApp::new().await;
    app.send_bytes(
        "/api/books/upload?title=%E3%83%86%E3%82%B9%E3%83%88",
        tiny_epub(),
    )
    .await;
    app.send(
        "POST",
        "/api/books/setup",
        json!({ "work": "テスト", "anchor": "少女は言った" }),
    )
    .await;

    let (_, preview) = app
        .send(
            "POST",
            "/api/books/preview",
            json!({ "work": "テスト", "anchor": "それきり黙った" }),
        )
        .await;
    let (status, skipped) = app
        .send(
            "POST",
            "/api/books/skip",
            json!({ "work": "テスト", "end": preview["found"]["end"] }),
        )
        .await;
    assert_eq!(status, 200, "{skipped}");

    // The bookmark moved and no day was credited for pages read before this
    // existed — the whole reason skip is not a session with zero minutes.
    assert_eq!(
        app.get("/api/books").await["books"][0]["position"],
        skipped["position"]
    );
    let sessions = app.get("/api/sessions").await;
    assert!(
        sessions["manual"].as_array().unwrap().is_empty(),
        "{sessions}"
    );

    // Reading on from there logs only what is left.
    let (_, preview) = app
        .send(
            "POST",
            "/api/books/preview",
            json!({ "work": "テスト", "anchor": "雨が降っていた" }),
        )
        .await;
    let (status, logged) = app
        .send(
            "POST",
            "/api/books/log",
            json!({ "work": "テスト", "end": preview["found"]["end"] }),
        )
        .await;
    assert_eq!(status, 200, "{logged}");
    assert_eq!(logged["session"]["chars"], 7, "雨が降っていた alone");
}

#[tokio::test]
async fn ingest_counts_the_characters_rather_than_trusting_the_source() {
    let app = TestApp::new().await;
    let (status, body) = app
        .send(
            "POST",
            "/api/lines",
            json!({
                "source": "vn",
                "lines": [{ "ts": today_start() + 3600.0, "text": "「ねえ、聞いてる？」" }]
            }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["ids"].as_array().unwrap().len(), 1);

    // Six counted codepoints, not the ten the line is written with: the
    // brackets and the punctuation are not reading.
    let s = app.get("/api/summary").await;
    assert_eq!(s["today"]["chars"], 6);
}

#[tokio::test]
async fn ingest_stamps_the_work_the_dashboard_is_reading() {
    let app = TestApp::new().await;
    kotodex_server::db::save_setting(&app.local, "current_work", "ハミダシクリエイティブ")
        .await
        .unwrap();

    app.send(
        "POST",
        "/api/lines",
        json!({ "lines": [{ "ts": today_start() + 3600.0, "text": "あいうえお" }] }),
    )
    .await;
    // A source may name its own instead — a bridge knows which video or book
    // it is on, and the dashboard's field is about the VN.
    app.send(
        "POST",
        "/api/lines",
        json!({
            "source": "ttsu",
            "work": "余生",
            "lines": [{ "ts": today_start() + 3700.0, "text": "かきくけこ" }]
        }),
    )
    .await;

    let works: Vec<String> = app
        .get("/api/works")
        .await
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["work"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(works.iter().any(|w| w == "ハミダシクリエイティブ"));
    assert!(works.iter().any(|w| w == "余生"));
}

#[tokio::test]
async fn ingest_drops_lines_while_capture_is_paused() {
    let app = TestApp::new().await;
    app.send("POST", "/api/capture/pause", json!({})).await;

    let (status, body) = app
        .send(
            "POST",
            "/api/lines",
            json!({ "lines": [{ "text": "あいうえお" }] }),
        )
        .await;
    // Accepted and dropped, not refused: a 4xx would make a source hold the
    // line and flush it the moment capture resumed.
    assert_eq!(status, 200);
    assert_eq!(body["paused"], true);
    assert_eq!(body["ids"].as_array().unwrap().len(), 0);
    assert_eq!(app.get("/api/summary").await["today"]["chars"], 0);
}

#[tokio::test]
async fn a_source_retracts_only_its_own_last_line() {
    let app = TestApp::new().await;
    let base = today_start() + 3600.0;
    app.send(
        "POST",
        "/api/lines",
        json!({ "source": "vn", "lines": [{ "ts": base, "text": "あいうえお" }] }),
    )
    .await;
    let (_, other) = app
        .send(
            "POST",
            "/api/lines",
            json!({ "source": "ttsu", "lines": [{ "ts": base + 10.0, "text": "かきくけこ" }] }),
        )
        .await;
    let other_id = other["ids"][0].clone();

    // The continuation case: the box that followed turned out to be the rest
    // of the same sentence, so the half already sent has to come back out.
    let (status, body) = app
        .send("POST", "/api/lines/retract", json!({ "source": "vn" }))
        .await;
    assert_eq!(status, 200);
    assert_ne!(body["id"], other_id, "another source's line is not touched");

    let lines = app.get("/api/lines/before?before=999999").await;
    let texts: Vec<&str> = lines["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, vec!["かきくけこ"]);
}

#[tokio::test]
async fn ingest_rejects_a_source_name_that_is_not_a_plain_token() {
    let app = TestApp::new().await;
    let (status, _) = app
        .send(
            "POST",
            "/api/lines",
            json!({ "source": "vn'; DROP", "lines": [] }),
        )
        .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn a_status_only_post_keeps_the_capture_badge_alive() {
    let app = TestApp::new().await;

    // A source with nothing to send still has something to say: the badge is
    // otherwise the browser's own connection, which stays perfectly healthy
    // while nothing at all is being captured.
    let (status, body) = app
        .send(
            "POST",
            "/api/lines",
            json!({ "lines": [], "status": { "attached": true, "pending": 3 } }),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(body["ids"].as_array().unwrap().len(), 0);

    let beat: serde_json::Value = serde_json::from_str(
        &kotodex_server::db::get_setting_raw(&app.local, "vn_logger_heartbeat")
            .await
            .unwrap()
            .expect("the beat is published where the badge reads it"),
    )
    .unwrap();
    assert_eq!(beat["ws"], true);
    assert_eq!(beat["pending"], 3);
    assert_eq!(beat["source"], "vn");
}

#[tokio::test]
async fn a_database_under_the_former_name_is_adopted_rather_than_left_behind() {
    let dir = std::env::temp_dir().join(format!("kotodex-rename-{}", now() as i64));
    std::fs::create_dir_all(&dir).unwrap();
    let former = dir.join("read-stats.db");
    let current = dir.join("kotodex.db");

    // A release that has already run, with a setting only it could hold.
    let pool = server_pool(former.to_str().unwrap()).await;
    kotodex_server::db::save_setting(&pool, "current_work", "余生")
        .await
        .unwrap();
    pool.close().await;

    let pool = server_pool(current.to_str().unwrap()).await;
    let settings = kotodex_server::db::load_settings(&pool).await.unwrap();
    assert_eq!(settings.current_work, "余生", "the history came across");
    assert!(!former.exists(), "the old name is gone, not duplicated");
    pool.close().await;
    std::fs::remove_dir_all(&dir).ok();
}

async fn server_pool(path: &str) -> sqlx::SqlitePool {
    kotodex_server::db::create_pool(path).await.unwrap()
}
