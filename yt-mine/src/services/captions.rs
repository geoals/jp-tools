//! YouTube's own captions, as the instant index into a video.
//!
//! Whisper is more accurate, but it walks the audio from 0:00 — a line at
//! 31:00 costs the whole 31 minutes of transcription before it can be read.
//! YouTube already has a timed transcript and yt-dlp fetches it in about a
//! second, no audio download at all. That is what the sentence list is built
//! from; `refine_window` replaces a stretch of it with whisper's reading when
//! a card is actually being made from it.

use std::future::Future;
use std::pin::Pin;

use serde::Deserialize;

use crate::models::TranscriptSegment;

#[derive(Debug, Clone)]
pub struct CaptionResult {
    pub video_title: String,
    pub video_duration: Option<f64>,
    pub segments: Vec<TranscriptSegment>,
    /// True when the uploader wrote the captions, false for YouTube's ASR.
    pub manual: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptionError {
    #[error("no Japanese captions for this video")]
    None,
    #[error("caption fetch failed: {0}")]
    Failed(String),
}

#[cfg_attr(test, mockall::automock)]
pub trait CaptionSource: Send + Sync {
    fn fetch(
        &self,
        url: String,
        output_dir: String,
    ) -> Pin<Box<dyn Future<Output = Result<CaptionResult, CaptionError>> + Send>>;
}

pub struct YtDlpCaptions;

impl CaptionSource for YtDlpCaptions {
    fn fetch(
        &self,
        url: String,
        output_dir: String,
    ) -> Pin<Box<dyn Future<Output = Result<CaptionResult, CaptionError>> + Send>> {
        Box::pin(async move {
            let id = crate::services::download::extract_video_id(&url)
                .ok_or(CaptionError::Failed("not a YouTube URL".into()))?;

            // Two passes rather than one, because manual and automatic captions
            // land under the same `ja` tag and the file gives no way to tell
            // them apart. Asking for manual first is the only way to prefer it.
            let (title, duration) = fetch_pass(&url, &output_dir, true).await?;
            if let Some(path) = find_json3(&output_dir, &id, &["ja", "ja-JP"]).await {
                return finish(path, title, duration, true).await;
            }

            fetch_pass(&url, &output_dir, false).await?;
            // `ja-orig` is the ASR's own output; `ja` beside it is that same
            // track machine-translated back into Japanese.
            let path = find_json3(&output_dir, &id, &["ja-orig", "ja"])
                .await
                .ok_or(CaptionError::None)?;
            finish(path, title, duration, false).await
        })
    }
}

async fn finish(
    path: String,
    video_title: String,
    video_duration: Option<f64>,
    manual: bool,
) -> Result<CaptionResult, CaptionError> {
    let body = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| CaptionError::Failed(format!("failed to read {path}: {e}")))?;
    tokio::fs::remove_file(&path).await.ok();

    let segments = parse_json3(&body)?;
    if segments.is_empty() {
        return Err(CaptionError::None);
    }
    Ok(CaptionResult {
        video_title,
        video_duration,
        segments,
        manual,
    })
}

/// One yt-dlp invocation. Returns the video's title and duration, which the
/// first pass is also the cheapest place to learn.
async fn fetch_pass(
    url: &str,
    output_dir: &str,
    manual: bool,
) -> Result<(String, Option<f64>), CaptionError> {
    let template = format!("{output_dir}/%(id)s");
    let write_flag = if manual {
        "--write-subs"
    } else {
        "--write-auto-subs"
    };
    let langs = if manual { "ja.*" } else { "ja-orig,ja" };

    let output = tokio::process::Command::new("yt-dlp")
        .args([
            "--no-update",
            "--skip-download",
            // `--print` implies `--simulate`, which silently writes no
            // subtitle file at all. This is what turns that back off.
            "--no-simulate",
            write_flag,
            "--sub-langs",
            langs,
            "--sub-format",
            "json3",
            "--print",
            "duration",
            "--print",
            "title",
            "-o",
            &template,
            url,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| CaptionError::Failed(format!("failed to run yt-dlp: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(CaptionError::Failed(format!("yt-dlp failed: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();
    let duration = lines.next().and_then(|s| s.parse::<f64>().ok());
    let title = lines.next().unwrap_or_default().to_string();
    Ok((title, duration))
}

async fn find_json3(dir: &str, id: &str, langs: &[&str]) -> Option<String> {
    for lang in langs {
        let path = format!("{dir}/{id}.{lang}.json3");
        if tokio::fs::try_exists(&path).await.unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

// --- json3 parsing ---

#[derive(Deserialize)]
struct Json3 {
    #[serde(default)]
    events: Vec<Event>,
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "tStartMs")]
    t_start_ms: f64,
    #[serde(rename = "aAppend", default)]
    append: u8,
    #[serde(default)]
    segs: Vec<Seg>,
}

#[derive(Deserialize)]
struct Seg {
    #[serde(default)]
    utf8: String,
    #[serde(rename = "tOffsetMs", default)]
    offset_ms: f64,
}

/// A sentence break is forced once a line is this long, at the next 、 —
/// the ASR drops 。 for stretches at a time, and a 37-second line is not a
/// sentence to mine.
const SOFT_CAP_CHARS: usize = 45;

/// Turn json3 cues into sentences with their own timings.
///
/// The cues are word-sized fragments, not lines, so they are flattened to
/// characters carrying an absolute timestamp each and re-split on Japanese
/// sentence punctuation. Rolling captions repeat the previous line as an
/// `aAppend` event; those are dropped.
pub fn parse_json3(body: &str) -> Result<Vec<TranscriptSegment>, CaptionError> {
    let doc: Json3 = serde_json::from_str(body)
        .map_err(|e| CaptionError::Failed(format!("malformed json3: {e}")))?;

    let mut chars: Vec<(char, f64)> = Vec::new();
    for event in &doc.events {
        if event.append == 1 {
            continue;
        }
        for seg in &event.segs {
            let text = seg.utf8.trim();
            // Sound annotations — [音楽], [拍手] — are not part of the line.
            if text.is_empty() || text.starts_with('[') {
                continue;
            }
            let at = (event.t_start_ms + seg.offset_ms) / 1000.0;
            chars.extend(text.chars().map(|c| (c, at)));
        }
    }

    let mut segments: Vec<TranscriptSegment> = Vec::new();
    let mut line: Vec<(char, f64)> = Vec::new();
    for (c, at) in chars {
        line.push((c, at));
        let hard = matches!(c, '。' | '？' | '！' | '?' | '!');
        let soft = c == '、' && line.len() >= SOFT_CAP_CHARS;
        if hard || soft {
            push_line(&mut segments, &mut line);
        }
    }
    push_line(&mut segments, &mut line);

    // A caption's last character is when it was *spoken*, not when the line
    // ended, so the end is stretched towards the next line's start — which is
    // what the audio clip on the card gets cut to. Half a second is the floor
    // even when the next line starts sooner than that: a clip shorter than
    // that is not worth playing.
    for i in 0..segments.len() {
        let next_start = segments.get(i + 1).map(|s| s.start);
        let s = &mut segments[i];
        let padded = s.end + 1.5;
        s.end = match next_start {
            Some(next) => padded.min(next),
            None => padded,
        }
        .max(s.start + 0.5);
    }

    Ok(segments)
}

fn push_line(out: &mut Vec<TranscriptSegment>, line: &mut Vec<(char, f64)>) {
    if line.is_empty() {
        return;
    }
    let text: String = line.iter().map(|(c, _)| c).collect();
    let start = line[0].1;
    let end = line[line.len() - 1].1;
    line.clear();
    if text.trim().is_empty() {
        return;
    }
    out.push(TranscriptSegment { start, end, text });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_cues_into_sentences_on_punctuation() {
        let body = r#"{"events":[
            {"tStartMs":1000,"segs":[{"utf8":"皆"},{"utf8":"さん","tOffsetMs":200},
                                     {"utf8":"こんにちは","tOffsetMs":400},{"utf8":"。","tOffsetMs":900}]},
            {"tStartMs":4000,"aAppend":1,"segs":[{"utf8":"\n"}]},
            {"tStartMs":5000,"segs":[{"utf8":"元気"},{"utf8":"ですか","tOffsetMs":300},{"utf8":"？","tOffsetMs":600}]}
        ]}"#;

        let segs = parse_json3(body).unwrap();
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0].text, "皆さんこんにちは。");
        assert_eq!(segs[1].text, "元気ですか？");
        assert!((segs[0].start - 1.0).abs() < 1e-9);
        assert!((segs[1].start - 5.0).abs() < 1e-9);
    }

    #[test]
    fn drops_sound_annotations() {
        let body =
            r#"{"events":[{"tStartMs":0,"segs":[{"utf8":"[音楽]"},{"utf8":"あ"},{"utf8":"。"}]}]}"#;
        let segs = parse_json3(body).unwrap();
        assert_eq!(segs[0].text, "あ。");
    }

    #[test]
    fn a_line_reaches_the_next_one_but_never_past_it() {
        let body = r#"{"events":[{"tStartMs":0,"segs":[{"utf8":"あ"},{"utf8":"。"}]},
                                  {"tStartMs":4000,"segs":[{"utf8":"い"},{"utf8":"。"}]}]}"#;
        let segs = parse_json3(body).unwrap();
        assert!((segs[0].end - 1.5).abs() < 1e-9);
        assert!(segs[0].end <= segs[1].start);
    }

    #[test]
    fn a_line_is_never_shorter_than_half_a_second() {
        let body = r#"{"events":[{"tStartMs":0,"segs":[{"utf8":"あ"},{"utf8":"。"}]},
                                  {"tStartMs":100,"segs":[{"utf8":"い"},{"utf8":"。"}]}]}"#;
        let segs = parse_json3(body).unwrap();
        assert!((segs[0].end - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_run_without_a_full_stop_breaks_at_a_comma() {
        let long = "あ".repeat(SOFT_CAP_CHARS);
        let body = format!(
            r#"{{"events":[{{"tStartMs":0,"segs":[{{"utf8":"{long}"}},{{"utf8":"、"}},{{"utf8":"い"}},{{"utf8":"。"}}]}}]}}"#
        );
        let segs = parse_json3(&body).unwrap();
        assert_eq!(segs.len(), 2);
        assert!(segs[0].text.ends_with('、'));
        assert_eq!(segs[1].text, "い。");
    }
}
