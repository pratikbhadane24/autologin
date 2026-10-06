//! Mobile browser session. Browser-based broker logins on Android/iOS need a
//! native WebView driver (next step of the mobile port); until then a session
//! can't be started and those brokers report it clearly. HTTP-only brokers
//! (e.g. Motilal Oswal) work on mobile already.

use std::path::{Path, PathBuf};

use regex::Regex;

use super::{DriverError, PageDriver, Selector};

const NOT_YET: &str =
    "Logging in through the broker's website isn't available on this phone yet. Motilal Oswal works today; other brokers are coming in an update.";

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub headless: bool,
    pub executable: Option<PathBuf>,
}

pub struct BrowserSession;

impl BrowserSession {
    pub async fn launch(_options: &LaunchOptions) -> Result<Self, DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }

    pub async fn new_page(&self, _intercept: Option<Regex>) -> Result<BrowserPage, DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }

    pub async fn close(self) {}
}

/// Never constructed until the WebView driver lands.
pub struct BrowserPage;

impl BrowserPage {
    pub async fn save_failure_artifacts(&self, _png: &Path, _html: &Path) -> Result<(), DriverError> {
        Ok(())
    }

    pub async fn close(self, _session: &BrowserSession) {}
}

impl PageDriver for BrowserPage {
    async fn goto(&self, _url: &str) -> Result<(), DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }
    async fn count(&self, _selector: &Selector) -> Result<usize, DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }
    async fn click(&self, _selector: &Selector, _index: usize) -> Result<(), DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }
    async fn fill(&self, _selector: &Selector, _index: usize, _value: &str) -> Result<(), DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }
    async fn type_text(&self, _selector: &Selector, _index: usize, _value: &str) -> Result<(), DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }
    async fn current_url(&self) -> Result<String, DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }
    async fn body_text(&self) -> Result<String, DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }
    async fn evaluate(&self, _js: &str) -> Result<(), DriverError> {
        Err(DriverError::Browser(NOT_YET.into()))
    }
    fn captured_callback(&self) -> Option<String> {
        None
    }
}
