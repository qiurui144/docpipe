//! 统一错误类型 + kebab-case 错误码 + HTTP 状态映射（spec §7）。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DocError {
    #[error("format-unsupported")]
    FormatUnsupported,
    #[error("file-too-large")]
    FileTooLarge,
    #[error("ocr-backend-unavailable: {0}")]
    OcrBackendUnavailable(String),
    #[error("mineru-timeout")]
    MineruTimeout,
    #[error("embedding-failed: {0}")]
    EmbeddingFailed(String),
    #[error("vector-store-error: {0}")]
    VectorStoreError(String),
    #[error("parse-empty-result")]
    ParseEmptyResult,
    #[error("locator-drift")]
    LocatorDrift,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("other: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, DocError>;

impl DocError {
    /// 稳定的 kebab-case 错误码（API 契约，不随 message 变化）。
    pub fn code(&self) -> &'static str {
        match self {
            DocError::FormatUnsupported => "format-unsupported",
            DocError::FileTooLarge => "file-too-large",
            DocError::OcrBackendUnavailable(_) => "ocr-backend-unavailable",
            DocError::MineruTimeout => "mineru-timeout",
            DocError::EmbeddingFailed(_) => "embedding-failed",
            DocError::VectorStoreError(_) => "vector-store-error",
            DocError::ParseEmptyResult => "parse-empty-result",
            DocError::LocatorDrift => "locator-drift",
            DocError::Io(_) => "io-error",
            DocError::Other(_) => "internal-error",
        }
    }

    pub fn http_status(&self) -> u16 {
        match self {
            DocError::FormatUnsupported => 400,
            DocError::FileTooLarge => 413,
            DocError::OcrBackendUnavailable(_) => 503,
            DocError::MineruTimeout => 504,
            DocError::EmbeddingFailed(_) => 502,
            DocError::VectorStoreError(_) => 500,
            DocError::ParseEmptyResult => 422,
            DocError::LocatorDrift => 422,
            DocError::Io(_) => 500,
            DocError::Other(_) => 500,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_and_statuses_match_spec() {
        assert_eq!(DocError::FormatUnsupported.code(), "format-unsupported");
        assert_eq!(DocError::FormatUnsupported.http_status(), 400);
        assert_eq!(DocError::FileTooLarge.code(), "file-too-large");
        assert_eq!(DocError::FileTooLarge.http_status(), 413);
        assert_eq!(DocError::OcrBackendUnavailable("x".into()).code(), "ocr-backend-unavailable");
        assert_eq!(DocError::OcrBackendUnavailable("x".into()).http_status(), 503);
        assert_eq!(DocError::MineruTimeout.code(), "mineru-timeout");
        assert_eq!(DocError::MineruTimeout.http_status(), 504);
        assert_eq!(DocError::EmbeddingFailed("x".into()).code(), "embedding-failed");
        assert_eq!(DocError::EmbeddingFailed("x".into()).http_status(), 502);
        assert_eq!(DocError::VectorStoreError("x".into()).code(), "vector-store-error");
        assert_eq!(DocError::VectorStoreError("x".into()).http_status(), 500);
        assert_eq!(DocError::ParseEmptyResult.code(), "parse-empty-result");
        assert_eq!(DocError::ParseEmptyResult.http_status(), 422);
        assert_eq!(DocError::LocatorDrift.code(), "locator-drift");
        assert_eq!(DocError::LocatorDrift.http_status(), 422);
    }
}
