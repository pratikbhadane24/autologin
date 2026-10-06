//! Removes the AutoLogin 1.x (Python/Briefcase) program after its data has
//! been migrated, so users don't end up with two AutoLogins.
//!
//! v1's MSI installed per-user with UpgradeCode
//! {A6D3467D-D77D-5786-AA4F-92D15AB50522} (read from the v1.0.24 MSI), while
//! v2 installs per-machine, so Windows Installer can't upgrade across the two
//! scopes. Instead v2 finds v1 in the user's uninstall list and removes it
//! silently. A per-user uninstall needs no administrator prompt.

/// Pure matching logic, shared by all platforms so it can be tested anywhere.
pub fn is_v1_entry(display_name: &str, display_version: &str, uninstall_string: &str) -> bool {
    display_name.trim() == "AutoLogin"
        && display_version.trim().starts_with("1.")
        && uninstall_string.to_ascii_lowercase().contains("msiexec")
}

/// Extract `{GUID}` from an MSI uninstall string like `MsiExec.exe /X{...}`.
pub fn product_code(uninstall_string: &str) -> Option<String> {
    let start = uninstall_string.find('{')?;
    let end = start + uninstall_string[start..].find('}')?;
    let code = &uninstall_string[start..=end];
    (code.len() == 38).then(|| code.to_string())
}

#[cfg(windows)]
pub fn remove_v1() -> Vec<String> {
    use std::process::Command;
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
    let Ok(root) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY) else { return Vec::new() };
    let codes: Vec<String> = root
        .enum_keys()
        .flatten()
        .filter_map(|name| root.open_subkey(name).ok())
        .filter_map(|key| {
            let get = |field: &str| key.get_value::<String, _>(field).unwrap_or_default();
            let uninstall = get("UninstallString");
            is_v1_entry(&get("DisplayName"), &get("DisplayVersion"), &uninstall)
                .then(|| product_code(&uninstall))
                .flatten()
        })
        .collect();

    codes
        .into_iter()
        .filter(|code| {
            let status = Command::new("msiexec").args(["/x", code, "/qn", "/norestart"]).status();
            match status {
                Ok(s) if s.success() => {
                    tracing::info!(%code, "removed AutoLogin 1.x");
                    true
                }
                Ok(s) => {
                    tracing::warn!(%code, exit = ?s.code(), "could not remove AutoLogin 1.x");
                    false
                }
                Err(error) => {
                    tracing::warn!(%error, "could not run msiexec");
                    false
                }
            }
        })
        .collect()
}

/// macOS/Linux: v1 shipped as a .dmg app / AppImage that the user removes;
/// nothing is registered system-wide to uninstall.
#[cfg(not(windows))]
pub fn remove_v1() -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_only_v1_msi_installs() {
        let uninstall = "MsiExec.exe /X{8B3C2F91-5344-4205-A968-68BB5431A27B}";
        assert!(is_v1_entry("AutoLogin", "1.0.24", uninstall));
        assert!(!is_v1_entry("AutoLogin", "2.0.0", uninstall), "never uninstall v2 itself");
        assert!(!is_v1_entry("AutoLogin Pro", "1.0.0", uninstall));
        assert!(!is_v1_entry("AutoLogin", "1.0.24", r"C:\x\uninstall.exe"));
    }

    #[test]
    fn extracts_product_code() {
        assert_eq!(
            product_code("MsiExec.exe /X{8B3C2F91-5344-4205-A968-68BB5431A27B}").as_deref(),
            Some("{8B3C2F91-5344-4205-A968-68BB5431A27B}")
        );
        assert_eq!(product_code("MsiExec.exe /X{bad}"), None);
        assert_eq!(product_code("nothing here"), None);
    }
}
