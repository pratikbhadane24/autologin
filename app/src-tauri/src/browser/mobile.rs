//! Mobile browser session: drives a plain system WebView owned by the native
//! `LoginWebViewPlugin` (Android). One login page exists at a time, because
//! Android WebViews share one cookie store per app; the page holds a slot that
//! serialises browser logins while HTTP-only brokers still run in parallel.
//! iOS has no native driver yet, so browser logins report that clearly there.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::Wry;
use tokio::sync::{Mutex, MutexGuard};

use super::{DriverError, PageDriver, Selector};

const LOCATE_JS: &str = include_str!("locate.js");
const FILL_JS: &str = include_str!("fill.js");
const CLICK_JS: &str = include_str!("click.js");
const NAVIGATION_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest any other WebView call may take; a dead page never answers.
const CALL_TIMEOUT: Duration = Duration::from_secs(15);
const NOT_YET: &str =
    "Logging in through the broker's website isn't available on this phone yet. Motilal Oswal works today; other brokers are coming in an update.";

static PLUGIN: OnceLock<PluginHandle<Wry>> = OnceLock::new();
static PAGE_SLOT: Mutex<()> = Mutex::const_new(());

/// Registers the native login WebView with Tauri.
pub fn init() -> TauriPlugin<Wry> {
    Builder::new("login-webview")
        .setup(|_app, api| {
            #[cfg(target_os = "android")]
            {
                let handle = api.register_android_plugin("trade.autologin.autologin", "LoginWebViewPlugin")?;
                let _ = PLUGIN.set(handle);
            }
            #[cfg(not(target_os = "android"))]
            let _ = api;
            Ok(())
        })
        .build()
}

fn browser_err(e: impl std::fmt::Display) -> DriverError {
    DriverError::Browser(e.to_string())
}

fn plugin() -> Result<&'static PluginHandle<Wry>, DriverError> {
    PLUGIN.get().ok_or_else(|| DriverError::Browser(NOT_YET.into()))
}

async fn call<T: DeserializeOwned>(command: &str, payload: impl Serialize) -> Result<T, DriverError> {
    let timed_out = || DriverError::Browser(format!("the login page stopped responding ({command})"));
    call_within(CALL_TIMEOUT, timed_out, command, payload).await
}

async fn call_within<T: DeserializeOwned>(
    limit: Duration,
    timed_out: impl FnOnce() -> DriverError,
    command: &str,
    payload: impl Serialize,
) -> Result<T, DriverError> {
    let request = plugin()?.run_mobile_plugin_async(command, payload);
    match tokio::time::timeout(limit, request).await {
        Ok(result) => result.map_err(browser_err),
        Err(_) => Err(timed_out()),
    }
}

#[derive(Deserialize)]
struct Value<T> {
    value: T,
}

#[derive(Debug, Clone, Default)]
pub struct LaunchOptions {
    pub headless: bool,
    pub executable: Option<PathBuf>,
}

pub struct BrowserSession {
    visible: bool,
}

impl BrowserSession {
    pub async fn launch(options: &LaunchOptions) -> Result<Self, DriverError> {
        plugin()?;
        tracing::info!(headless = options.headless, "login WebView ready");
        Ok(Self { visible: !options.headless })
    }

    pub async fn new_page(&self, intercept: Option<Regex>) -> Result<BrowserPage, DriverError> {
        let slot = PAGE_SLOT.lock().await;
        #[derive(Serialize)]
        struct Open {
            visible: bool,
            intercept: Option<String>,
        }
        let open = Open { visible: self.visible, intercept: intercept.map(|r| r.as_str().to_string()) };
        call::<IgnoredAny>("open", open).await?;
        let page = BrowserPage { slot: Some(slot) };
        // A 0x0 page (screen off, no layout) would make every field "hidden".
        if let Ok(viewport) = page.eval::<String>("innerWidth + 'x' + innerHeight").await {
            tracing::debug!(%viewport, "login page opened");
        }
        Ok(page)
    }

    pub async fn close(self) {}
}

/// Holds the login slot until the page is closed. Dropping it without
/// `close` (e.g. a cancelled run) still closes the WebView, so a logged-in
/// broker session never lingers, and keeps the slot until that's done.
pub struct BrowserPage {
    slot: Option<MutexGuard<'static, ()>>,
}

impl Drop for BrowserPage {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            tauri::async_runtime::spawn(async move {
                close_webview().await;
                drop(slot);
            });
        }
    }
}

async fn close_webview() {
    if let Err(error) = call::<IgnoredAny>("close", ()).await {
        tracing::debug!(%error, "login WebView close");
    }
}

impl BrowserPage {
    async fn eval<T: DeserializeOwned>(&self, script: &str) -> Result<T, DriverError> {
        #[derive(Serialize)]
        struct Script<'a> {
            script: &'a str,
        }
        let result: Value<String> = call("eval", Script { script }).await?;
        serde_json::from_str(&result.value).map_err(|e| DriverError::Browser(format!("unexpected page result: {e}")))
    }

    /// Tags the matching element and returns its token.
    async fn tag(&self, selector: &Selector, index: usize) -> Result<String, DriverError> {
        let token = format!("m{}", next_token());
        let count = self.locate(selector, index, Some(&token)).await?;
        if index >= count {
            return Err(DriverError::NoElement { selector: selector.to_string(), index });
        }
        Ok(token)
    }

    async fn locate(&self, selector: &Selector, index: usize, token: Option<&str>) -> Result<usize, DriverError> {
        let call = format!(
            "{LOCATE_JS}({}, {}, {index}, {})",
            json(selector.kind()),
            json(selector.query()),
            token.map_or("null".to_string(), json),
        );
        self.eval(&call).await
    }

    async fn run_on_tag(&self, script: &str, selector: &Selector, index: usize, args: &str) -> Result<(), DriverError> {
        let token = self.tag(selector, index).await?;
        let done: bool = self.eval(&format!("{script}({}{args})", json(&token))).await?;
        if done {
            Ok(())
        } else {
            Err(DriverError::NoElement { selector: selector.to_string(), index })
        }
    }

    pub async fn save_failure_artifacts(&self, png_path: &Path, html_path: &Path) -> Result<(), DriverError> {
        #[derive(Serialize)]
        struct Shot {
            path: String,
        }
        call::<IgnoredAny>("screenshot", Shot { path: png_path.to_string_lossy().into_owned() }).await?;
        let html: String = self.eval("document.documentElement ? document.documentElement.outerHTML : ''").await?;
        tokio::fs::write(html_path, html).await.map_err(browser_err)
    }

    pub async fn close(mut self, _session: &BrowserSession) {
        let slot = self.slot.take();
        close_webview().await;
        drop(slot);
    }
}

fn json(value: &str) -> String {
    serde_json::to_string(value).expect("strings always serialize")
}

fn next_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl PageDriver for BrowserPage {
    async fn goto(&self, url: &str) -> Result<(), DriverError> {
        #[derive(Serialize)]
        struct Load<'a> {
            url: &'a str,
        }
        let timed_out = || DriverError::NavigationTimeout(url.split('?').next().unwrap_or(url).to_string());
        match call_within::<IgnoredAny>(NAVIGATION_TIMEOUT, timed_out, "load", Load { url }).await {
            Ok(_) => Ok(()),
            Err(_) if self.captured_callback().is_some() => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn count(&self, selector: &Selector) -> Result<usize, DriverError> {
        self.locate(selector, 0, None).await
    }

    async fn click(&self, selector: &Selector, index: usize) -> Result<(), DriverError> {
        self.run_on_tag(CLICK_JS, selector, index, "").await
    }

    async fn fill(&self, selector: &Selector, index: usize, value: &str) -> Result<(), DriverError> {
        self.run_on_tag(FILL_JS, selector, index, &format!(", {}", json(value))).await
    }

    async fn type_text(&self, selector: &Selector, index: usize, value: &str) -> Result<(), DriverError> {
        self.run_on_tag(CLICK_JS, selector, index, "").await?;
        #[derive(Serialize)]
        struct Text<'a> {
            text: &'a str,
        }
        call::<IgnoredAny>("typeText", Text { text: value }).await.map(|_| ())
    }

    async fn current_url(&self) -> Result<String, DriverError> {
        Ok(call::<Value<String>>("url", ()).await?.value)
    }

    async fn body_text(&self) -> Result<String, DriverError> {
        self.eval("document.body ? document.body.innerText : ''").await
    }

    async fn evaluate(&self, js: &str) -> Result<(), DriverError> {
        self.eval::<IgnoredAny>(js).await.map(|_| ())
    }

    fn captured_callback(&self) -> Option<String> {
        // The trait is synchronous; the native side answers this without
        // touching the page, so a short blocking call is fine here.
        let handle = PLUGIN.get()?;
        match handle.run_mobile_plugin::<Value<Option<String>>>("captured", ()) {
            Ok(result) => result.value,
            Err(error) => {
                tracing::debug!(%error, "could not read captured callback");
                None
            }
        }
    }
}
