use std::future::Future;
use std::pin::Pin;

/// OCR seam, mirroring yt-mine's `Transcriber`. Takes an encoded image crop
/// (a pre-cropped text region — recognition only, not detection) and returns
/// the recognized text.
#[cfg_attr(test, mockall::automock)]
pub trait OcrEngine: Send + Sync {
    fn recognize(
        &self,
        image_bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<String, OcrError>> + Send>>;
}

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("ocr failed: {0}")]
    Failed(String),
}

/// Client for manga-ocr-service (`POST /ocr` multipart → `{ "text": ... }`).
pub struct MangaOcrEngine {
    url: String,
    client: reqwest::Client,
}

impl MangaOcrEngine {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
        }
    }
}

impl OcrEngine for MangaOcrEngine {
    fn recognize(
        &self,
        image_bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<String, OcrError>> + Send>> {
        let client = self.client.clone();
        let url = format!("{}/ocr", self.url.trim_end_matches('/'));

        Box::pin(async move {
            let part = reqwest::multipart::Part::bytes(image_bytes)
                .file_name("crop.jpg")
                .mime_str("image/jpeg")
                .map_err(|e| OcrError::Failed(format!("failed to build request: {e}")))?;
            let form = reqwest::multipart::Form::new().part("image", part);

            let response = client
                .post(&url)
                .multipart(form)
                .send()
                .await
                .map_err(|e| OcrError::Failed(format!("request to OCR service failed: {e}")))?;

            if !response.status().is_success() {
                return Err(OcrError::Failed(format!(
                    "OCR service returned {}",
                    response.status()
                )));
            }

            let body: serde_json::Value = response
                .json()
                .await
                .map_err(|e| OcrError::Failed(format!("failed to parse OCR response: {e}")))?;

            body["text"]
                .as_str()
                .map(|s| s.to_owned())
                .ok_or_else(|| OcrError::Failed("OCR response missing 'text'".into()))
        })
    }
}
