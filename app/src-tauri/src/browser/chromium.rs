//! `PageDriver` over Chrome/Edge via the DevTools protocol (chromiumoxide).
//!
//! One browser process per run; each account gets its own browser context
//! (separate cookies/storage, like an incognito window).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::browser::BrowserContextId;
use chromiumoxide::cdp::browser_protocol::inspector::{EnableParams as InspectorEnable, EventDetached};
use chromiumoxide::cdp::browser_protocol::fetch::{
    ContinueRequestParams, EnableParams, EventRequestPaused, FailRequestParams, RequestPattern, RequestStage,
};
use chromiumoxide::cdp::browser_protocol::network::{ErrorReason, ResourceType};
use chromiumoxide::cdp::browser_protocol::page::CaptureScreenshotFormat;
use chromiumoxide::cdp::browser_protocol::target::{CreateBrowserContextParams, CreateTargetParams};
use chromiumoxide::page::{Page, ScreenshotParams};
use futures::StreamExt;
use regex::Regex;
use tokio::task::JoinHandle;

use super::{DriverError, PageDriver, Selector};

const LOCATE_JS: &str = include_str!("locate.js");
const FILL_JS: &str = include_str!("fill.js");
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 900;

static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn browser_err(e: impl std::fmt::Display) -> DriverError {
    DriverError::Browser(e.to_string())
}

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    /// Hide the browser window. Scheduled runs default to true.
    pub headless: bool,
    /// Use this Chrome/Edge binary instead of auto-detection.
    pub executable: Option<PathBuf>,
}

async fn watch_targets(browser: &Browser) {
    use chromiumoxide::cdp::browser_protocol::target::{EventTargetCrashed, EventTargetDestroyed};
    if let Ok(mut destroyed) = browser.event_listener::<EventTargetDestroyed>().await {
        tokio::spawn(async move {
            while let Some(event) = destroyed.next().await {
                tracing::debug!(target = %event.target_id.as_ref(), "Chrome target destroyed");
            }
        });
    }
    if let Ok(mut crashed) = browser.event_listener::<EventTargetCrashed>().await {
        tokio::spawn(async move {
            while let Some(event) = crashed.next().await {
                tracing::warn!(target = %event.target_id.as_ref(), status = %event.status, code = event.error_code, "Chrome page crashed");
            }
        });
    }
}

pub struct ChromeSession {
    browser: Browser,
    handler: JoinHandle<()>,
    user_agent: String,
    /// Fresh Chrome profile for this session only, deleted on drop. Sharing
    /// one profile (chromiumoxide's default) stops a second Chrome from
    /// starting and lets sessions interfere; it would also keep cookies.
    profile: tempfile::TempDir,
}

impl ChromeSession {
    pub async fn launch(options: &LaunchOptions) -> Result<Self, DriverError> {
        let profile = tempfile::Builder::new()
            .prefix("autologin-chrome-")
            .tempdir()
            .map_err(|e| DriverError::Browser(format!("could not create a browser profile folder: {e}")))?;
        let mut builder = BrowserConfig::builder()
            .user_data_dir(profile.path())
            .window_size(WINDOW_WIDTH, WINDOW_HEIGHT)
            .launch_timeout(LAUNCH_TIMEOUT)
            .hide();
        builder = if options.headless { builder.new_headless_mode() } else { builder.with_head().viewport(None) };
        if let Some(path) = &options.executable {
            builder = builder.chrome_executable(path);
        }
        // Development: Chrome writes chrome_debug.log into the profile folder.
        if crate::app::dev::chrome_log_target().is_some() {
            builder = builder.args(["enable-logging", "v=1"]);
        }
        let config = builder.build().map_err(|e| {
            DriverError::Browser(format!("Chrome or Edge was not found ({e}). Install Google Chrome and retry."))
        })?;

        let (browser, mut handler) = Browser::launch(config).await.map_err(browser_err)?;
        let handler = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                if let Err(error) = event {
                    tracing::trace!(%error, "devtools handler event error");
                }
            }
        });

        watch_targets(&browser).await;
        // Headless Chrome announces itself in the UA; brokers may block that.
        let user_agent = browser.user_agent().await.map_err(browser_err)?.replace("HeadlessChrome", "Chrome");
        tracing::info!(headless = options.headless, "browser launched");
        Ok(Self { browser, handler, user_agent, profile })
    }

    /// A fresh, isolated tab. Navigations matching `intercept` are blocked
    /// and recorded instead of loaded (the auth code in them is single-use).
    pub async fn new_page(&self, intercept: Option<Regex>) -> Result<ChromePage, DriverError> {
        let context_id = self
            .browser
            .create_browser_context(CreateBrowserContextParams::default())
            .await
            .map_err(browser_err)?;
        let params = CreateTargetParams::builder()
            .url("about:blank")
            .browser_context_id(context_id.clone())
            .build()
            .map_err(browser_err)?;
        let page = self.browser.new_page(params).await.map_err(browser_err)?;
        page.enable_stealth_mode_with_agent(&self.user_agent).await.map_err(browser_err)?;
        watch_lifecycle(&page).await;

        let captured = Arc::new(Mutex::new(None));
        let interceptor = match intercept {
            Some(pattern) => Some(start_interception(&page, pattern, Arc::clone(&captured)).await?),
            None => None,
        };
        Ok(ChromePage { page, context_id, captured, interceptor })
    }

    pub fn profile_dir(&self) -> &Path {
        self.profile.path()
    }

    /// Close Chrome, wait for it to exit, then delete the profile folder.
    pub async fn close(mut self) {
        if let Err(error) = self.browser.close().await {
            tracing::debug!(%error, "browser close");
        }
        let _ = self.browser.wait().await;
        self.handler.abort();
        if let Some(target) = crate::app::dev::chrome_log_target() {
            // One file per session so a later run doesn't overwrite the evidence.
            let stamped = target.with_extension(format!("{}.log", chrono::Local::now().format("%H%M%S")));
            let _ = std::fs::copy(self.profile.path().join("chrome_debug.log"), stamped);
        }
        if let Err(error) = self.profile.close() {
            tracing::debug!(%error, "could not delete browser profile folder");
        }
    }
}

/// Log why Chrome detaches from a page (closed, crashed, replaced...). These
/// are the only events that make later commands fail with "receiver is gone".
async fn watch_lifecycle(page: &Page) {
    let target = page.target_id().as_ref().to_string();
    tracing::debug!(%target, "page opened");
    if let Err(error) = page.execute(InspectorEnable::default()).await {
        tracing::debug!(%error, "inspector enable failed");
        return;
    }
    match page.event_listener::<EventDetached>().await {
        Ok(mut detached) => {
            tokio::spawn(async move {
                while let Some(event) = detached.next().await {
                    tracing::warn!(%target, reason = %event.reason, "Chrome detached from page");
                }
            });
        }
        Err(error) => tracing::debug!(%error, "inspector listener failed"),
    }
}

async fn start_interception(
    page: &Page,
    pattern: Regex,
    captured: Arc<Mutex<Option<String>>>,
) -> Result<JoinHandle<()>, DriverError> {
    let mut paused = page.event_listener::<EventRequestPaused>().await.map_err(browser_err)?;
    let documents_only = RequestPattern {
        url_pattern: Some("*".to_string()),
        resource_type: Some(ResourceType::Document),
        request_stage: Some(RequestStage::Request),
    };
    page.execute(EnableParams::builder().pattern(documents_only).build()).await.map_err(browser_err)?;

    let page = page.clone();
    Ok(tokio::spawn(async move {
        while let Some(event) = paused.next().await {
            let url = event.request.url.clone();
            let result = if pattern.is_match(&url) {
                tracing::info!("callback redirect intercepted");
                if let Ok(mut slot) = captured.lock() {
                    *slot = Some(url);
                }
                page.execute(FailRequestParams::new(event.request_id.clone(), ErrorReason::Aborted)).await.map(|_| ())
            } else {
                page.execute(ContinueRequestParams::new(event.request_id.clone())).await.map(|_| ())
            };
            if let Err(error) = result {
                tracing::debug!(%error, "interception reply failed");
            }
        }
    }))
}

pub struct ChromePage {
    page: Page,
    context_id: BrowserContextId,
    captured: Arc<Mutex<Option<String>>>,
    interceptor: Option<JoinHandle<()>>,
}

impl ChromePage {
    /// Tag match `index` of `selector` and return a CSS selector for it.
    async fn tag(&self, selector: &Selector, index: usize) -> Result<String, DriverError> {
        let token = NEXT_TOKEN.fetch_add(1, Ordering::Relaxed).to_string();
        let count = self.locate(selector, index, Some(&token)).await?;
        if index >= count {
            return Err(DriverError::NoElement { selector: selector.to_string(), index });
        }
        Ok(format!("[data-autologin=\"{token}\"]"))
    }

    async fn locate(&self, selector: &Selector, index: usize, token: Option<&str>) -> Result<usize, DriverError> {
        let call = format!(
            "{LOCATE_JS}({}, {}, {index}, {})",
            json(selector.kind()),
            json(selector.query()),
            token.map_or("null".to_string(), json),
        );
        let result = self.page.evaluate(call).await.map_err(browser_err)?;
        result.into_value::<usize>().map_err(browser_err)
    }

    /// Save a full-page PNG and the page HTML, for failure reports.
    pub async fn save_failure_artifacts(&self, png_path: &Path, html_path: &Path) -> Result<(), DriverError> {
        let params = ScreenshotParams::builder().format(CaptureScreenshotFormat::Png).full_page(true).build();
        self.page.save_screenshot(params, png_path).await.map_err(browser_err)?;
        let html = self.page.content().await.map_err(browser_err)?;
        tokio::fs::write(html_path, html).await.map_err(browser_err)
    }

    pub async fn close(self, session: &ChromeSession) {
        if let Some(task) = self.interceptor {
            task.abort();
        }
        let _ = self.page.close().await;
        if let Err(error) = session.browser.dispose_browser_context(self.context_id).await {
            tracing::debug!(%error, "dispose browser context");
        }
    }
}

fn json(value: &str) -> String {
    serde_json::to_string(value).expect("strings always serialize")
}

impl PageDriver for ChromePage {
    async fn goto(&self, url: &str) -> Result<(), DriverError> {
        match tokio::time::timeout(NAVIGATION_TIMEOUT, self.page.goto(url)).await {
            Ok(Ok(_)) => Ok(()),
            // A goto straight into an intercepted callback is aborted on purpose.
            Ok(Err(_)) if self.captured_callback().is_some() => Ok(()),
            Ok(Err(error)) => Err(browser_err(error)),
            Err(_) => Err(DriverError::NavigationTimeout(url.split('?').next().unwrap_or(url).to_string())),
        }
    }

    async fn count(&self, selector: &Selector) -> Result<usize, DriverError> {
        self.locate(selector, 0, None).await
    }

    async fn click(&self, selector: &Selector, index: usize) -> Result<(), DriverError> {
        let css = self.tag(selector, index).await?;
        let element = self.page.find_element(css).await.map_err(browser_err)?;
        element.scroll_into_view().await.map_err(browser_err)?;
        element.click().await.map_err(browser_err)?;
        Ok(())
    }

    async fn fill(&self, selector: &Selector, index: usize, value: &str) -> Result<(), DriverError> {
        let css = self.tag(selector, index).await?;
        let token = css.trim_start_matches("[data-autologin=\"").trim_end_matches("\"]");
        let call = format!("{FILL_JS}({}, {})", json(token), json(value));
        let filled = self.page.evaluate(call).await.map_err(browser_err)?.into_value::<bool>().map_err(browser_err)?;
        if filled {
            Ok(())
        } else {
            Err(DriverError::NoElement { selector: selector.to_string(), index })
        }
    }

    async fn type_text(&self, selector: &Selector, index: usize, value: &str) -> Result<(), DriverError> {
        let css = self.tag(selector, index).await?;
        let element = self.page.find_element(css).await.map_err(browser_err)?;
        element.click().await.map_err(browser_err)?;
        element.type_str(value).await.map_err(browser_err)?;
        Ok(())
    }

    async fn current_url(&self) -> Result<String, DriverError> {
        Ok(self.page.url().await.map_err(browser_err)?.unwrap_or_default())
    }

    async fn body_text(&self) -> Result<String, DriverError> {
        let result = self
            .page
            .evaluate("document.body ? document.body.innerText : ''")
            .await
            .map_err(browser_err)?;
        result.into_value::<String>().map_err(browser_err)
    }

    async fn evaluate(&self, js: &str) -> Result<(), DriverError> {
        self.page.evaluate(js).await.map(|_| ()).map_err(browser_err)
    }

    fn captured_callback(&self) -> Option<String> {
        self.captured.lock().ok().and_then(|slot| slot.clone())
    }
}
