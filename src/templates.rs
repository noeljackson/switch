use std::collections::BTreeMap;
use std::path::Path;

use crate::paths::{expand_path, file_or_dir_exists};

/// A known application: where to look for it, and how to name its profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppTemplate {
    pub detect_paths: Vec<String>,
    pub auth_path: String,
    pub pattern: String,
    pub description: String,
}

fn template(
    detect_paths: &[&str],
    auth_path: &str,
    pattern: &str,
    description: &str,
) -> AppTemplate {
    AppTemplate {
        detect_paths: detect_paths.iter().map(|s| s.to_string()).collect(),
        auth_path: auth_path.to_string(),
        pattern: pattern.to_string(),
        description: description.to_string(),
    }
}

/// The built-in application templates.
///
/// Folder-backed apps keep their profiles under `~/.switch/profiles/<app>/`
/// rather than inside the folder being backed up. The original templates
/// nested them (`~/.ssh` backed up to `~/.ssh/profiles/...`), which meant the
/// second `add` walked the first backup and recursed.
pub fn app_templates() -> BTreeMap<String, AppTemplate> {
    let entries = [
        (
            "codex",
            template(
                &["~/.codex/auth.json"],
                "~/.codex/auth.json",
                "{auth_path}.{name}.switch",
                "Codex authentication file",
            ),
        ),
        (
            "claude",
            template(
                &["~/.claude/config.json"],
                "~/.claude/config.json",
                "{auth_path}.{name}.switch",
                "Claude configuration file",
            ),
        ),
        (
            "claudecode",
            template(
                &["~/.claude/settings.json", "~/.claude/config.json"],
                "~/.claude/settings.json",
                "{auth_path}.{name}.switch",
                "Claude Code configuration file",
            ),
        ),
        (
            "antigravity",
            template(
                // macOS and Linux standard locations first, then the legacy one.
                &[
                    "~/Library/Application Support/Antigravity",
                    "~/.config/Antigravity",
                    "~/.antigravity",
                ],
                "~/Library/Application Support/Antigravity",
                "~/.switch/profiles/antigravity/{name}",
                "Antigravity configuration folder",
            ),
        ),
        (
            "vscode",
            template(
                &["~/.vscode/User", "~/Library/Application Support/Code/User"],
                "~/.vscode/User",
                "~/.switch/profiles/vscode/{name}",
                "VSCode user settings folder",
            ),
        ),
        (
            "cursor",
            template(
                &["~/.cursor", "~/Library/Application Support/Cursor"],
                "~/.cursor",
                "~/.switch/profiles/cursor/{name}",
                "Cursor configuration folder",
            ),
        ),
        (
            "ssh",
            template(
                &["~/.ssh"],
                "~/.ssh",
                "~/.switch/profiles/ssh/{name}",
                "SSH configuration folder",
            ),
        ),
        (
            "git",
            template(
                &["~/.gitconfig"],
                "~/.gitconfig",
                "{auth_path}.{name}.switch",
                "Git configuration file",
            ),
        ),
    ];

    entries
        .into_iter()
        .map(|(name, tpl)| (name.to_string(), tpl))
        .collect()
}

pub fn app_template(name: &str) -> Option<AppTemplate> {
    app_templates().remove(name)
}

/// Finds which known applications are installed. Port of `DetectApplications`.
///
/// The first detect path that exists wins. When the template's own auth path is
/// missing, the detected path takes its place — which is how an app installed
/// in a secondary location still gets a usable config path.
pub fn detect_applications(home: &Path) -> BTreeMap<String, AppTemplate> {
    let mut found = BTreeMap::new();

    for (name, tpl) in app_templates() {
        for detect in &tpl.detect_paths {
            let detected = expand_path(home, detect);
            if !file_or_dir_exists(&detected) {
                continue;
            }

            let mut resolved = tpl.clone();
            if !file_or_dir_exists(expand_path(home, &tpl.auth_path)) {
                resolved.auth_path = detected;
            }
            found.insert(name.clone(), resolved);
            break;
        }
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::clean;

    // Port of TestDetectApplications.
    #[test]
    fn detect_applications_finds_installed_apps() {
        let home = tempfile::tempdir().unwrap();

        // Only the second vscode detect path exists, not ~/.vscode/User.
        let vscode_alt = home
            .path()
            .join("Library")
            .join("Application Support")
            .join("Code")
            .join("User");
        std::fs::create_dir_all(&vscode_alt).unwrap();

        let claude = home.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(claude.parent().unwrap()).unwrap();
        std::fs::write(&claude, b"{}").unwrap();

        let found = detect_applications(home.path());
        assert!(found.contains_key("claudecode"), "claudecode not detected");
        assert!(found.contains_key("vscode"), "vscode not detected");

        // With the default auth path absent, the detected path is used instead.
        let got = clean(&expand_path(home.path(), &found["vscode"].auth_path));
        let want = clean(&vscode_alt.to_string_lossy());
        assert_eq!(got, want, "vscode auth_path not set to the detected path");
    }

    #[test]
    fn detect_keeps_template_auth_path_when_it_exists() {
        let home = tempfile::tempdir().unwrap();
        let codex = home.path().join(".codex").join("auth.json");
        std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
        std::fs::write(&codex, b"{}").unwrap();

        let found = detect_applications(home.path());
        assert_eq!(found["codex"].auth_path, "~/.codex/auth.json");
    }

    #[test]
    fn nothing_is_detected_in_an_empty_home() {
        let home = tempfile::tempdir().unwrap();
        assert!(detect_applications(home.path()).is_empty());
    }

    /// A backup that lives inside the folder it backs up cannot work: the copy
    /// walks its own output, and restoring would delete the other profiles.
    #[test]
    fn no_template_nests_its_backups_inside_the_config_folder() {
        let home = Path::new("/home/tester");
        for (name, tpl) in app_templates() {
            let auth = expand_path(home, &tpl.auth_path);
            let backup = crate::paths::resolve_switch_pattern(&tpl.pattern, &auth, "p", home);
            assert!(
                !crate::fsops::overlaps(&auth, &backup),
                "template {name}: backup {backup} overlaps config path {auth}"
            );
        }
    }

    #[test]
    fn templates_match_the_documented_set() {
        let names: Vec<String> = app_templates().into_keys().collect();
        assert_eq!(
            names,
            vec![
                "antigravity",
                "claude",
                "claudecode",
                "codex",
                "cursor",
                "git",
                "ssh",
                "vscode",
            ]
        );
    }
}
