use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// The contents of `~/.switch.toml`.
///
/// The on-disk shape is fixed by the Go original's struct tags and must not
/// change: a `[default]` table followed by one `[apps.<name>]` table per
/// configured application. Field order here determines the emitted order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub default: DefaultConfig,
    /// Ordered so that `switch list` is deterministic. Go ranged over a map,
    /// which randomised the order on every run.
    #[serde(default)]
    pub apps: BTreeMap<String, AppConfig>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultConfig {
    #[serde(default)]
    pub config: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub current: String,
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default)]
    pub auth_path: String,
    #[serde(default)]
    pub switch_pattern: String,
}

/// Reads the config file, or creates it seeded with the `codex` default when it
/// does not exist yet. Port of `loadConfig`, including the write-on-first-run.
pub fn load_config(path: &Path) -> Result<Config> {
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let config = Config {
                default: DefaultConfig {
                    config: "codex".to_string(),
                },
                apps: BTreeMap::new(),
            };
            save_config(path, &config)?;
            return Ok(config);
        }
        Err(e) => return Err(Error::new(format!("read config: {}: {e}", path.display()))),
    };

    let text = String::from_utf8_lossy(&data);
    toml::from_str(&text).map_err(|e| Error::new(format!("parse config: {e}")))
}

/// Writes the config file.
///
/// The bytes are staged in a sibling file and renamed into place, so an
/// interrupted write cannot leave a truncated `~/.switch.toml` — which would
/// lose every configured profile. The Go version truncated in place.
pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    let text = toml::to_string(config).map_err(|e| Error::new(format!("encode config: {e}")))?;

    crate::fsops::write_atomic(path, text.as_bytes())
        .map_err(|e| Error::new(format!("write config: {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Port of TestLoadSaveConfig_RoundTrip.
    #[test]
    fn load_save_config_round_trip() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".switch.toml");

        let mut config = load_config(&path).unwrap();
        config.default.config = "codex".to_string();
        config.apps.insert(
            "codex".to_string(),
            AppConfig {
                current: "u1".to_string(),
                accounts: vec!["u1".to_string(), "u2".to_string()],
                auth_path: "~/.codex/auth.json".to_string(),
                switch_pattern: "{auth_path}.{name}.switch".to_string(),
            },
        );
        save_config(&path, &config).unwrap();

        let reloaded = load_config(&path).unwrap();
        assert_eq!(reloaded.default.config, "codex");
        assert!(
            reloaded.apps.contains_key("codex"),
            "apps map missing codex"
        );
        assert_eq!(reloaded, config);
    }

    // Port of TestLoadConfig_ParseError.
    #[test]
    fn load_config_parse_error() {
        let home = tempfile::tempdir().unwrap();
        let bad = home.path().join(".switch.toml");
        std::fs::write(&bad, b"not=toml=here\n[apps\n").unwrap();

        let err = load_config(&bad).unwrap_err();
        assert!(
            err.to_string().contains("parse config"),
            "expected a parse error, got {err}"
        );
    }

    // Port of TestLoadConfig_ReadError.
    #[test]
    fn load_config_read_error() {
        let home = tempfile::tempdir().unwrap();
        // A directory can be neither read nor treated as missing.
        let err = load_config(home.path()).unwrap_err();
        assert!(
            err.to_string().contains("read config"),
            "expected a read error, got {err}"
        );
    }

    // Port of TestSaveConfig_ErrorOnDirectoryPath.
    #[test]
    fn save_config_error_on_directory_path() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join("confdir");
        std::fs::create_dir_all(&dir).unwrap();

        let err = save_config(&dir, &Config::default()).unwrap_err();
        assert!(
            err.to_string().contains("write config"),
            "expected a write error, got {err}"
        );
    }

    #[test]
    fn load_config_creates_file_when_missing() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".switch.toml");

        let config = load_config(&path).unwrap();
        assert_eq!(config.default.config, "codex");
        assert!(config.apps.is_empty());
        assert!(path.exists(), "config file should have been created");
    }

    #[test]
    fn partial_config_fills_in_defaults() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".switch.toml");
        // Exactly the shape one of the Go CLI tests writes by hand.
        std::fs::write(&path, b"[default]\nconfig=\"\"\n\n[apps]\n").unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(config.default.config, "");
        assert!(config.apps.is_empty());
    }

    #[test]
    fn app_table_omitting_keys_uses_zero_values() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".switch.toml");
        std::fs::write(&path, b"[apps.codex]\nauth_path = \"~/x\"\n").unwrap();

        let config = load_config(&path).unwrap();
        let app = &config.apps["codex"];
        assert_eq!(app.auth_path, "~/x");
        assert_eq!(app.current, "");
        assert_eq!(app.switch_pattern, "");
        assert!(app.accounts.is_empty());
        assert_eq!(config.default.config, "");
    }

    /// The exact bytes the Go binary writes, to guarantee an existing
    /// `~/.switch.toml` keeps working after the port.
    const GO_WRITTEN_CONFIG: &str = r#"[default]
  config = "codex"

[apps]
  [apps.codex]
    current = "work"
    accounts = ["personal", "work"]
    auth_path = "~/.codex/auth.json"
    switch_pattern = "{auth_path}.{name}.switch"
"#;

    #[test]
    fn reads_a_config_written_by_the_go_implementation() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".switch.toml");
        std::fs::write(&path, GO_WRITTEN_CONFIG.as_bytes()).unwrap();

        let config = load_config(&path).unwrap();
        assert_eq!(config.default.config, "codex");
        let codex = &config.apps["codex"];
        assert_eq!(codex.current, "work");
        assert_eq!(codex.accounts, vec!["personal", "work"]);
        assert_eq!(codex.auth_path, "~/.codex/auth.json");
        assert_eq!(codex.switch_pattern, "{auth_path}.{name}.switch");
    }

    #[test]
    fn rewriting_a_go_config_preserves_every_value() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".switch.toml");
        std::fs::write(&path, GO_WRITTEN_CONFIG.as_bytes()).unwrap();

        let original = load_config(&path).unwrap();
        save_config(&path, &original).unwrap();
        assert_eq!(load_config(&path).unwrap(), original);
    }

    #[test]
    fn emitted_toml_uses_the_apps_table_layout() {
        let config = Config {
            default: DefaultConfig {
                config: "codex".to_string(),
            },
            apps: BTreeMap::from([(
                "codex".to_string(),
                AppConfig {
                    current: "work".to_string(),
                    accounts: vec!["personal".to_string(), "work".to_string()],
                    auth_path: "~/.codex/auth.json".to_string(),
                    switch_pattern: "{auth_path}.{name}.switch".to_string(),
                },
            )]),
        };
        let text = toml::to_string(&config).unwrap();
        assert!(text.contains("[default]"), "{text}");
        assert!(text.contains("[apps.codex]"), "{text}");
        assert!(text.contains("config = \"codex\""), "{text}");
        assert!(
            text.contains("auth_path = \"~/.codex/auth.json\""),
            "{text}"
        );
    }
}
