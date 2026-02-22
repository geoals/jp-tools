use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Downloading,
    Transcribing,
    Done,
    Error,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Downloading => "downloading",
            Self::Transcribing => "transcribing",
            Self::Done => "done",
            Self::Error => "error",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
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
    pub video_title: Option<String>,
    pub audio_path: Option<String>,
    pub video_path: Option<String>,
    pub status: JobStatus,
    pub error_message: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct Sentence {
    pub id: i64,
    pub job_id: i64,
    pub text: String,
    pub start_time: f64,
    pub end_time: f64,
    pub created_at: String,
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
