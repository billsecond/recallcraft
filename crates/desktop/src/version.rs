//! What `--version` and About say.

pub fn version() -> &'static str {
    option_env!("RECALLCRAFT_VERSION").filter(|v| !v.is_empty()).unwrap_or(recallcraft_model::VERSION)
}

/// `RecallCraft 0.1.0`.
pub fn line(program: &str) -> String {
    format!("{program} {}", version())
}
