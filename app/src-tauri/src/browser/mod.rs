//! Browser automation behind a small trait, so the step engine can be tested
//! with a fake page and a mobile WebView driver can be added later.

// Desktop drives Chrome/Edge over CDP; mobile drives the system WebView
// through a native plugin. Both expose the same session/page API.
#[cfg(desktop)]
pub mod chromium;
#[cfg(mobile)]
pub mod mobile;

#[cfg(desktop)]
pub use chromium::{ChromePage as BrowserPage, ChromeSession as BrowserSession, LaunchOptions};
#[cfg(mobile)]
pub use mobile::{BrowserPage, BrowserSession, LaunchOptions};

use std::fmt;

use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    Css(String),
    XPath(String),
    /// Smallest visible element whose text contains this (case-insensitive).
    Text(String),
}

impl Selector {
    pub fn parse(raw: &str) -> Self {
        if let Some(xpath) = raw.strip_prefix("xpath=") {
            Self::XPath(xpath.to_string())
        } else if let Some(text) = raw.strip_prefix("text=") {
            Self::Text(text.to_string())
        } else {
            Self::Css(raw.to_string())
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Css(_) => "css",
            Self::XPath(_) => "xpath",
            Self::Text(_) => "text",
        }
    }

    fn query(&self) -> &str {
        match self {
            Self::Css(q) | Self::XPath(q) | Self::Text(q) => q,
        }
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.kind(), self.query())
    }
}

#[derive(Debug, Error)]
pub enum DriverError {
    #[error("browser error: {0}")]
    Browser(String),
    #[error("no element #{index} for {selector}")]
    NoElement { selector: String, index: usize },
    #[error("navigation timed out: {0}")]
    NavigationTimeout(String),
}

/// One page (tab) in an isolated browser context.
#[allow(async_fn_in_trait)]
pub trait PageDriver {
    async fn goto(&self, url: &str) -> Result<(), DriverError>;
    /// Number of visible elements matching `selector`.
    async fn count(&self, selector: &Selector) -> Result<usize, DriverError>;
    async fn click(&self, selector: &Selector, index: usize) -> Result<(), DriverError>;
    /// Set the value programmatically (fires input/change events).
    async fn fill(&self, selector: &Selector, index: usize, value: &str) -> Result<(), DriverError>;
    /// Focus and send real key events, for inputs that ignore `fill`.
    async fn type_text(&self, selector: &Selector, index: usize, value: &str) -> Result<(), DriverError>;
    async fn current_url(&self) -> Result<String, DriverError>;
    /// Visible text of the page body.
    async fn body_text(&self) -> Result<String, DriverError>;
    async fn evaluate(&self, js: &str) -> Result<(), DriverError>;
    /// The callback URL caught by interception, if one has been seen.
    fn captured_callback(&self) -> Option<String>;
}
