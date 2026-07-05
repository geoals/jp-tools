use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use tracing::error;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("not found")]
    NotFound,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("image error: {0}")]
    Image(String),

    #[error("ocr error: {0}")]
    Ocr(String),

    #[error("export error: {0}")]
    Export(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            AppError::Io(e) => {
                error!(error = %e, "io error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
            AppError::Image(e) => {
                error!(error = %e, "image error");
                (StatusCode::INTERNAL_SERVER_ERROR, "image processing failed".to_string())
            }
            AppError::Ocr(e) => {
                error!(error = %e, "ocr error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "OCR failed. Is manga-ocr-service running?".to_string(),
                )
            }
            AppError::Export(e) => {
                error!(error = %e, "export error");
                (StatusCode::INTERNAL_SERVER_ERROR, "export failed".to_string())
            }
        };

        (status, message).into_response()
    }
}
