pub mod page;
pub mod browser;

pub use browser::Browser;
pub use page::Page;

/// Core error type for the minibrowser
#[derive(Debug, thiserror::Error)]
pub enum MbError {
    #[error("Network error: {0}")]
    Network(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("JS execution error: {0}")]
    Js(String),
    #[error("Storage error: {0}")]
    Storage(String),
    #[error("Navigation error: {0}")]
    Navigation(String),
}

pub type Result<T> = std::result::Result<T, MbError>;
