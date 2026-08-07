use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    /// Pulling YouTube's own captions — a second or two, and the whole video.
    Fetching,
    Downloading,
    Transcribing,
    Done,
    Error,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Fetching => "fetching",
            Self::Downloading => "downloading",
            Self::Transcribing => "transcribing",
            Self::Done => "done",
            Self::Error => "error",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "fetching" => Some(Self::Fetching),
            "downloading" => Some(Self::Downloading),
            "transcribing" => Some(Self::Transcribing),
            "done" => Some(Self::Done),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Error)
    }
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: i64,
    pub youtube_url: String,
    pub video_id: Option<String>,
    pub video_title: Option<String>,
    pub audio_path: Option<String>,
    pub video_path: Option<String>,
    pub status: JobStatus,
    pub error_message: Option<String>,
    pub created_at: String,
    pub segments_found: i64,
    pub video_duration: Option<f64>,
    /// `running`, `done`, or an error message — the state of the whisper pass
    /// over one window, which runs after the job itself is finished.
    pub refine_state: Option<String>,
    pub refine_at: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Sentence {
    pub id: i64,
    pub job_id: i64,
    pub text: String,
    pub start_time: f64,
    pub end_time: f64,
    pub created_at: String,
    /// `captions` or `whisper` — which transcript this line came from, and so
    /// whether sharpening the window around it would change anything.
    pub source: String,
    /// The window this line was transcribed from, when it has one. Media for
    /// the card is cut out of it; a line still on captions has none until an
    /// export fetches one.
    pub clip_path: Option<String>,
    pub clip_audio_path: Option<String>,
    pub clip_start: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TranscriptSegment {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_status_roundtrip() {
        let statuses = [
            JobStatus::Pending,
            JobStatus::Fetching,
            JobStatus::Downloading,
            JobStatus::Transcribing,
            JobStatus::Done,
            JobStatus::Error,
        ];

        for status in &statuses {
            let s = status.as_str();
            let parsed = JobStatus::from_str(s).unwrap();
            assert_eq!(&parsed, status);
        }
    }

    #[test]
    fn job_status_from_str_unknown_returns_none() {
        assert_eq!(JobStatus::from_str("unknown"), None);
    }

    #[test]
    fn job_status_is_terminal() {
        assert!(!JobStatus::Pending.is_terminal());
        assert!(!JobStatus::Downloading.is_terminal());
        assert!(!JobStatus::Transcribing.is_terminal());
        assert!(JobStatus::Done.is_terminal());
        assert!(JobStatus::Error.is_terminal());
    }
}
