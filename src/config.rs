use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// User-overridable configuration for `gprr`. Loaded from
/// `$XDG_CONFIG_HOME/gprr/config.toml` (default) or `--config <PATH>`.
///
/// The defaults are *also* what gets written on first run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub refresh: RefreshConfig,
    #[serde(default)]
    pub keys: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct UiConfig {
    /// Built-in palette: `"dark"` (default) or `"light"`. Unknown values fall
    /// back to dark. `--no-color` / `NO_COLOR` override this at runtime.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Animation intensity: `"full"` (default), `"subtle"`, or `"off"`.
    /// `--no-color` / `NO_COLOR` force `"off"` at runtime regardless of this
    /// setting.
    #[serde(default = "default_animations")]
    pub animations: String,
}

fn default_theme() -> String {
    "dark".to_string()
}

fn default_animations() -> String {
    "full".to_string()
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: "dark".to_string(),
            animations: default_animations(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
pub struct RefreshConfig {
    /// Dashboard refresh interval, seconds.
    pub dashboard: u64,
    /// PR list refresh interval, seconds.
    pub pr_list: u64,
    /// PR detail refresh interval, seconds.
    pub pr_detail: u64,
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            dashboard: 60,
            pr_list: 60,
            pr_detail: 30,
        }
    }
}

impl ConfigFile {
    /// Load config from `override_path` if `Some`, otherwise from
    /// `$XDG_CONFIG_HOME/gprr/config.toml` (creating it with defaults if
    /// it does not exist). Returns `default()` on any read/parse error.
    #[must_use]
    pub fn load(override_path: Option<&Path>) -> Self {
        if let Some(p) = override_path {
            return Self::read_or_default(p);
        }
        let path = default_config_path();
        if !path.exists() {
            let _ = write_default(&path);
        }
        Self::read_or_default(&path)
    }

    fn read_or_default(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }
}

fn default_config_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("gprr").join("config.toml");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("gprr")
            .join("config.toml");
    }
    // No `XDG_CONFIG_HOME` or `HOME` (typically Windows): fall back to the
    // platform's native config directory, mirroring `logging::log_dir`.
    if let Some(proj) = directories::ProjectDirs::from("", "", "gprr") {
        return proj.config_dir().join("config.toml");
    }
    PathBuf::from(".").join("gprr").join("config.toml")
}

fn write_default(path: &Path) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(&ConfigFile::default()).map_err(std::io::Error::other)?;
    std::fs::write(path, content)
}

#[cfg(test)]
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    /// Tests that mutate `XDG_CONFIG_HOME`/`HOME` must serialize — env is
    /// process-global. Otherwise concurrent test threads race the value.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn load_returns_default_when_override_path_missing() {
        let cfg = ConfigFile::load(Some(Path::new("/definitely/does/not/exist.toml")));
        assert_eq!(cfg, ConfigFile::default());
    }

    #[test]
    fn load_writes_default_to_xdg_path_when_none_passed_and_missing() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempdir().unwrap();
        let saved_xdg = std::env::var_os("XDG_CONFIG_HOME");
        // SAFETY: protected by ENV_LOCK above.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path());
        }

        let path = dir.path().join("gprr").join("config.toml");
        assert!(!path.exists(), "precondition: not written yet");

        let cfg = ConfigFile::load(None);
        assert_eq!(cfg, ConfigFile::default());
        assert!(path.exists(), "load(None) writes default config");
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("[refresh]"),
            "default config has [refresh] section"
        );

        // SAFETY: protected by ENV_LOCK above.
        unsafe {
            match saved_xdg {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    #[test]
    fn load_parses_refresh_and_keys_sections() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[refresh]\ndashboard = 120\npr_list = 90\npr_detail = 45\n\n[keys]\nquit = \"Q\"\n",
        )
        .unwrap();
        let cfg = ConfigFile::load(Some(&path));
        assert_eq!(cfg.refresh.dashboard, 120);
        assert_eq!(cfg.refresh.pr_list, 90);
        assert_eq!(cfg.refresh.pr_detail, 45);
        assert_eq!(cfg.keys.get("quit").map(String::as_str), Some("Q"));
    }

    #[test]
    fn load_parses_ui_theme_and_defaults_to_dark() {
        assert_eq!(ConfigFile::default().ui.theme, "dark");
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ui]\ntheme = \"light\"\n").unwrap();
        let cfg = ConfigFile::load(Some(&path));
        assert_eq!(cfg.ui.theme, "light");
    }

    #[test]
    fn animations_defaults_to_full() {
        assert_eq!(ConfigFile::default().ui.animations, "full");
    }

    #[test]
    fn animations_override_is_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[ui]\nanimations = \"subtle\"\n").unwrap();
        let cfg = ConfigFile::load(Some(&path));
        assert_eq!(cfg.ui.animations, "subtle");
    }

    #[test]
    fn malformed_config_falls_back_to_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "this is = not valid toml ][[[[").unwrap();
        let cfg = ConfigFile::load(Some(&path));
        assert_eq!(cfg, ConfigFile::default());
    }
}
