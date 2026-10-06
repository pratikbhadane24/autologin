//! `autologin://` links, so Cirrus can hand accounts to the app in one click:
//!
//!   autologin://import?d=<base64url of the signed "Copy for AutoLogin" text>
//!   autologin://open
//!
//! An import link carries exactly what the clipboard would; it goes through
//! the same signature/expiry checks (`paste::parse`) and the user still
//! confirms the accounts in the bulk-add screen. Nothing is saved directly.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use tauri::{AppHandle, Emitter, Manager};
use url::Url;

use super::state::AppState;

pub const SCHEME: &str = "autologin";
pub const IMPORT_EVENT: &str = "cirrus-import";
/// Same cap as a clipboard paste (paste::MAX_PASTE_BYTES), base64-expanded.
const MAX_DATA_CHARS: usize = 350 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum LinkAction {
    /// Signed copy text to show in the bulk-add screen.
    Import(String),
    /// Just bring the window forward.
    Open,
}

/// Understand an `autologin://` URL; anything else is ignored.
pub fn parse_link(url: &Url) -> Option<LinkAction> {
    if url.scheme() != SCHEME {
        return None;
    }
    match url.host_str().unwrap_or_default() {
        "open" | "" => Some(LinkAction::Open),
        "import" => {
            let data = url.query_pairs().find(|(k, _)| k == "d").map(|(_, v)| v.into_owned())?;
            if data.len() > MAX_DATA_CHARS {
                return None;
            }
            let bytes = URL_SAFE_NO_PAD.decode(data.trim_end_matches('=')).ok()?;
            String::from_utf8(bytes).ok().map(LinkAction::Import)
        }
        _ => None,
    }
}

/// Handle links from the OS: show the window and pass imports to the UI.
/// The UI may not be loaded yet (app launched by the link), so the import is
/// also kept until the UI collects it with `take_pending_import`.
pub fn handle(app: &AppHandle, urls: Vec<Url>) {
    for url in urls {
        let Some(action) = parse_link(&url) else {
            tracing::warn!(scheme = url.scheme(), "ignored unrecognised app link");
            continue;
        };
        super::tray::show(app);
        if let LinkAction::Import(text) = action {
            tracing::info!("accounts received from Cirrus via app link");
            if let Ok(mut pending) = app.state::<AppState>().pending_import.lock() {
                *pending = Some(text.clone());
            }
            if let Err(error) = app.emit(IMPORT_EVENT, ()) {
                tracing::debug!(%error, "could not notify UI of app link import");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(s: &str) -> Option<LinkAction> {
        parse_link(&Url::parse(s).unwrap())
    }

    #[test]
    fn decodes_import_payload() {
        let text = r#"{"autologin":1,"kid":"k","payload":"p","sig":"s"}"#;
        let url = format!("autologin://import?d={}", URL_SAFE_NO_PAD.encode(text));
        assert_eq!(link(&url), Some(LinkAction::Import(text.to_string())));
    }

    #[test]
    fn open_and_bare_scheme_just_focus_the_app() {
        assert_eq!(link("autologin://open"), Some(LinkAction::Open));
        assert_eq!(link("autologin://"), Some(LinkAction::Open));
    }

    #[test]
    fn rejects_other_schemes_actions_and_bad_data() {
        assert_eq!(link("https://import?d=abc"), None);
        assert_eq!(link("autologin://delete-everything"), None);
        assert_eq!(link("autologin://import"), None);
        assert_eq!(link("autologin://import?d=%%%"), None);
        let huge = format!("autologin://import?d={}", "A".repeat(MAX_DATA_CHARS + 4));
        assert_eq!(link(&huge), None);
    }
}
