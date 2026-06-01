//! Top-level error type. Variants are added as each layer needs them.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Config(String),
    #[error("auth: {0}")]
    Auth(String),
    #[error("github: {0}")]
    GitHub(String),
    #[error("git: {0}")]
    Git(String),
    #[error("internal: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_variant_message() {
        let e = Error::Config("missing key".into());
        assert_eq!(e.to_string(), "config: missing key");
    }
}
