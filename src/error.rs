use serde_json::json;
use std::fmt;

#[derive(Debug)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}
impl Error {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn json(&self) -> serde_json::Value {
        json!({"ok":false,"error":{"code":self.code,"message":self.message}})
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::new("io_error", e.to_string())
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::new("invalid_json", e.to_string())
    }
}
pub type Result<T> = std::result::Result<T, Error>;

impl From<windows::core::Error> for Error {
    fn from(e: windows::core::Error) -> Self {
        Self::new(
            "windows_error",
            format!("{} (HRESULT 0x{:08X})", e.message(), e.code().0 as u32),
        )
    }
}
