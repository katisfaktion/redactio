use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use std::{fmt, io};

/// The complete public error payload. Never retain OS messages or input paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppError {
    #[serde(deserialize_with = "safe_code")]
    pub code: String,
    pub retryable: bool,
}

impl AppError {
    pub(crate) fn new(code: &'static str) -> Self {
        Self {
            code: code.into(),
            retryable: false,
        }
    }
}

impl From<io::Error> for AppError {
    fn from(error: io::Error) -> Self {
        #[cfg(windows)]
        if matches!(error.raw_os_error(), Some(32 | 33)) {
            return Self {
                code: "file_busy".into(),
                retryable: true,
            };
        }
        let code = match error.kind() {
            io::ErrorKind::NotFound => "path_unavailable",
            io::ErrorKind::PermissionDenied => "permission_denied",
            io::ErrorKind::AlreadyExists => "path_exists",
            _ => "storage_io",
        };
        Self::new(code)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.code)
    }
}
impl std::error::Error for AppError {}

fn safe_code<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    let value = String::deserialize(deserializer)?;
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_lowercase() || (index > 0 && (byte.is_ascii_digit() || byte == b'_'))
        })
    {
        return Err(D::Error::custom("invalid safe error code"));
    }
    Ok(value)
}
