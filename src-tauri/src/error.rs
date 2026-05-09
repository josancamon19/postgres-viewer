use std::error::Error;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl From<tokio_postgres::Error> for AppError {
    fn from(value: tokio_postgres::Error) -> Self {
        let code = value
            .as_db_error()
            .map(|error| error.code().code().to_string())
            .unwrap_or_else(|| "postgres_error".to_string());

        Self::new(code, error_chain(&value))
    }
}

fn error_chain(error: &dyn Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();

    while let Some(next) = source {
        let detail = next.to_string();
        if !message.contains(&detail) {
            message.push_str(": ");
            message.push_str(&detail);
        }
        source = next.source();
    }

    message
}

impl From<native_tls::Error> for AppError {
    fn from(value: native_tls::Error) -> Self {
        Self::new("tls_error", value.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::new("json_error", value.to_string())
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        Self::new("io_error", value.to_string())
    }
}

impl From<security_framework::base::Error> for AppError {
    fn from(value: security_framework::base::Error) -> Self {
        Self::new("keychain_error", value.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(value: reqwest::Error) -> Self {
        Self::new("network_error", value.to_string())
    }
}
