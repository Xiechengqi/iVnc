//! Errors of the management API. The English `message` stays in the `error`
//! field, which the iVnc CLI and scripts read; `code` and `params` let the
//! console show the error in the user's language.
use serde_json::{json, Map, Value};

/// Code of failures without a category of their own, e.g. database errors.
const INTERNAL: &str = "internal";

#[derive(Clone, Debug)]
pub struct ApiError {
    /// Stable snake_case identifier of the error.
    pub code: &'static str,
    /// Dynamic parts of the message. `detail` carries text that cannot be
    /// translated, such as a network error; the console shows it after the message.
    pub params: Map<String, Value>,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            params: Map::new(),
            message: message.into(),
        }
    }

    pub fn param(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }

    /// Gives a failure without a category the `code`, keeping its text as the
    /// `detail`. Errors that have a code keep it.
    pub fn or(self, code: &'static str) -> Self {
        if self.code != INTERNAL {
            return self;
        }
        let detail = self.message.clone();
        Self { code, ..self }.param("detail", detail)
    }

    /// The JSON response body: `{"error", "code", "params"?}`.
    pub fn body(&self) -> Value {
        let mut body = json!({"error": self.message, "code": self.code});
        if !self.params.is_empty() {
            body["params"] = Value::Object(self.params.clone());
        }
        body
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<String> for ApiError {
    fn from(message: String) -> Self {
        Self::new(INTERNAL, message)
    }
}

impl From<&str> for ApiError {
    fn from(message: &str) -> Self {
        Self::new(INTERNAL, message)
    }
}

impl From<ApiError> for String {
    fn from(error: ApiError) -> Self {
        error.message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_failures_without_a_code_take_a_category() {
        let failed = ApiError::from("spawn failed").or("app_start_failed");
        assert_eq!(
            failed.body(),
            json!({"error": "spawn failed", "code": "app_start_failed", "params": {"detail": "spawn failed"}})
        );
        let running = ApiError::new("app_already_running", "App is already running");
        assert_eq!(
            running.or("app_start_failed").body(),
            json!({"error": "App is already running", "code": "app_already_running"})
        );
    }
}
