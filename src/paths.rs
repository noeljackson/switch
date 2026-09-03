use std::path::{Path, PathBuf};

/// Resolves the user's home directory from the environment.
///
/// Mirrors `getHomeDir` in the Go original, which deliberately preferred the
/// environment over the platform API so that tests could redirect it. On
/// Windows `USERPROFILE` wins, then `HOME` on every platform.
pub fn home_from_env() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(home) = non_empty_env("USERPROFILE") {
        return Some(home);
    }
    non_empty_env("HOME")
}

fn non_empty_env(key: &str) -> Option<PathBuf> {
    match std::env::var(key) {
        Ok(v) if !v.is_empty() => Some(PathBuf::from(v)),
        _ => None,
    }
}

/// Expands a leading `~`, `~/` or `~\` to the home directory and normalises
/// separators to forward slashes.
///
/// Port of `expandPath`. The order matters and is preserved: backslashes are
/// folded to slashes first so the tilde forms can be matched uniformly, then
/// the path is cleaned lexically. Forms such as `~user` are left alone, exactly
/// as the Go version left them.
pub fn expand_path(home: &Path, p: &str) -> String {
    if p.is_empty() {
        return String::new();
    }

    let mut p = p.replace('\\', "/");

    if p.starts_with('~') {
        let home = to_slash(home);
        if p == "~" {
            p = home;
        } else if let Some(rest) = p.strip_prefix("~/") {
            p = join_slash(&home, rest);
        }
    }

    clean(&p)
}

/// Substitutes `{auth_path}` and `{name}` into a switch pattern, then expands
/// the result. Port of `resolveSwitchPattern`.
pub fn resolve_switch_pattern(pattern: &str, auth_path: &str, name: &str, home: &Path) -> String {
    let resolved = pattern
        .replace("{auth_path}", auth_path)
        .replace("{name}", name)
        .replace('\\', "/");
    expand_path(home, &resolved)
}

pub fn file_or_dir_exists(path: impl AsRef<Path>) -> bool {
    std::fs::metadata(path.as_ref()).is_ok()
}

pub fn is_folder(path: impl AsRef<Path>) -> bool {
    std::fs::metadata(path.as_ref())
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

fn to_slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Joins two slash-separated fragments the way `filepath.Join` does: empty
/// elements are skipped entirely rather than contributing a separator.
fn join_slash(a: &str, b: &str) -> String {
    if a.is_empty() {
        return b.to_string();
    }
    if b.is_empty() {
        return a.to_string();
    }
    format!("{a}/{b}")
}

/// Lexical path cleaning equivalent to Go's `filepath.Clean` followed by
/// `filepath.ToSlash`, operating purely on slash-separated input.
///
/// Rust's `Path::components` is not a substitute: it does not resolve `..` and
/// normalises differently, so the rules are implemented directly here. Doing it
/// on strings also means Linux and Windows agree, which the Windows-style path
/// tests depend on.
pub fn clean(path: &str) -> String {
    if path.is_empty() {
        return ".".to_string();
    }

    let (volume, rest) = split_volume(path);
    let rooted = rest.starts_with('/');

    let mut parts: Vec<&str> = Vec::new();
    for part in rest.split('/') {
        match part {
            "" | "." => continue,
            ".." => match parts.last() {
                Some(&last) if last != ".." => {
                    parts.pop();
                }
                _ => {
                    // A `..` that would escape a rooted path is discarded;
                    // otherwise it has to be kept.
                    if !rooted {
                        parts.push("..");
                    }
                }
            },
            _ => parts.push(part),
        }
    }

    let joined = parts.join("/");
    let cleaned = if rooted {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    };

    if volume.is_empty() {
        cleaned
    } else {
        format!("{volume}{cleaned}")
    }
}

/// Splits off a Windows drive prefix such as `C:`.
///
/// Only Windows has a volume concept in `filepath`, so on other platforms this
/// is intentionally a no-op and `C:/../x` cleans to `x`, just as Go does there.
fn split_volume(p: &str) -> (&str, &str) {
    #[cfg(windows)]
    {
        let b = p.as_bytes();
        if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
            return p.split_at(2);
        }
    }
    ("", p)
}

/// Where switch keeps its own data, including profile backups for
/// folder-backed apps: `~/.switch`.
pub fn switch_dir(home: &Path) -> String {
    expand_path(home, "~/.switch")
}

/// The safe backup pattern for an app: a per-app directory under
/// [`switch_dir`], well away from the config being backed up.
pub fn default_backup_pattern(app: &str) -> String {
    format!("~/.switch/profiles/{app}/{{name}}")
}

/// Last element of a path. Equivalent to `filepath.Base`.
pub fn base(p: &str) -> String {
    if p.is_empty() {
        return ".".to_string();
    }
    let p = p.replace('\\', "/");
    let trimmed = p.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    match trimmed.rfind('/') {
        Some(i) => trimmed[i + 1..].to_string(),
        None => trimmed.to_string(),
    }
}

/// Everything but the last element of a path. Equivalent to `filepath.Dir`.
pub fn dir(p: &str) -> String {
    let p = p.replace('\\', "/");
    match p.rfind('/') {
        Some(i) => clean(&p[..=i]),
        None => ".".to_string(),
    }
}

/// Joins path elements and cleans the result. Equivalent to `filepath.Join`.
pub fn join_paths(parts: &[&str]) -> String {
    let joined = parts
        .iter()
        .filter(|part| !part.is_empty())
        .copied()
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        String::new()
    } else {
        clean(&joined)
    }
}

/// Uppercases the first character of every word.
///
/// Reproduces the (deprecated) `strings.Title` the Go code used, including its
/// notion of a word boundary: digits, ASCII letters and `_` do not separate
/// words, so `my_app` becomes `My_app` rather than `My_App`.
pub fn title_case(s: &str) -> String {
    let mut prev = ' ';
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_separator(prev) {
            out.extend(c.to_uppercase());
        } else {
            out.push(c);
        }
        prev = c;
    }
    out
}

fn is_separator(c: char) -> bool {
    if (c as u32) <= 0x7f {
        return !(c.is_ascii_alphanumeric() || c == '_');
    }
    if c.is_alphabetic() || c.is_numeric() {
        return false;
    }
    c.is_whitespace()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Port of TestExpandAndResolve.
    #[test]
    fn expand_and_resolve() {
        let home = Path::new("/home/tester");
        assert_eq!(expand_path(home, "~/file.txt"), "/home/tester/file.txt");

        let p = resolve_switch_pattern(
            "{auth_path}.{name}.switch",
            "/home/tester/.codex/auth.json",
            "alice",
            home,
        );
        assert!(
            p.ends_with(".codex/auth.json.alice.switch"),
            "resolveSwitchPattern unexpected: {p}"
        );
    }

    // Port of TestExpandAndResolve_WindowsLikePaths.
    #[test]
    fn expand_and_resolve_windows_like_paths() {
        let home = Path::new("/home/tester");

        assert_eq!(
            expand_path(home, "~\\sub\\file.txt"),
            "/home/tester/sub/file.txt"
        );

        let auth = expand_path(home, "~\\.codex\\auth.json");
        let out = resolve_switch_pattern("{auth_path}\\{name}.switch", &auth, "alice", home);
        assert!(
            out.ends_with(".codex/auth.json/alice.switch")
                || out.ends_with(".codex/auth.json.alice.switch"),
            "resolveSwitchPattern windows-like unexpected: {out}"
        );

        let norm = expand_path(home, "C:\\Users\\me\\file.txt");
        assert!(
            !norm.contains('\\'),
            "expected forward slashes, got {norm:?}"
        );
    }

    #[test]
    fn expand_path_tilde_forms() {
        let home = Path::new("/home/tester");
        assert_eq!(expand_path(home, ""), "");
        assert_eq!(expand_path(home, "~"), "/home/tester");
        // `~user` is not a supported form and is left as written.
        assert_eq!(expand_path(home, "~other/x"), "~other/x");
        // An empty home collapses away instead of rooting the path.
        assert_eq!(expand_path(Path::new(""), "~/file.txt"), "file.txt");
    }

    #[test]
    fn clean_matches_go_semantics() {
        assert_eq!(clean(""), ".");
        assert_eq!(clean("."), ".");
        assert_eq!(clean("/"), "/");
        assert_eq!(clean("//a//b"), "/a/b");
        assert_eq!(clean("a/./b"), "a/b");
        assert_eq!(clean("a/b/../c"), "a/c");
        assert_eq!(clean("a/.."), ".");
        assert_eq!(clean("/.."), "/");
        assert_eq!(clean("/a/../.."), "/");
        assert_eq!(clean("../a"), "../a");
        assert_eq!(clean("../../a"), "../../a");
        assert_eq!(clean("a/b/"), "a/b");
    }

    #[test]
    fn title_case_matches_go_strings_title() {
        assert_eq!(title_case("codex"), "Codex");
        assert_eq!(title_case("claudecode"), "Claudecode");
        assert_eq!(title_case(""), "");
        // `_` is not a separator for Go's strings.Title, but `-` and space are.
        assert_eq!(title_case("my_app"), "My_app");
        assert_eq!(title_case("my-app"), "My-App");
        assert_eq!(title_case("two words"), "Two Words");
    }

    #[test]
    fn default_backup_pattern_is_per_app() {
        assert_eq!(
            default_backup_pattern("ssh"),
            "~/.switch/profiles/ssh/{name}"
        );
        assert_eq!(
            resolve_switch_pattern(
                &default_backup_pattern("ssh"),
                "/home/tester/.ssh",
                "work",
                Path::new("/home/tester")
            ),
            "/home/tester/.switch/profiles/ssh/work"
        );
    }

    #[test]
    fn base_and_dir_match_go_filepath() {
        assert_eq!(base(""), ".");
        assert_eq!(base("/"), "/");
        assert_eq!(base("auth.json"), "auth.json");
        assert_eq!(base("/a/b/auth.json"), "auth.json");
        assert_eq!(base("/a/b/"), "b");
        assert_eq!(base("/home/u/.confdir"), ".confdir");

        assert_eq!(dir(""), ".");
        assert_eq!(dir("auth.json"), ".");
        assert_eq!(dir("/a/b/auth.json"), "/a/b");
        assert_eq!(dir("/a"), "/");
        assert_eq!(dir("a/b/"), "a/b");
    }

    #[test]
    fn join_paths_matches_go_filepath_join() {
        assert_eq!(join_paths(&["/a", "b", "c"]), "/a/b/c");
        assert_eq!(
            join_paths(&[".", "profiles", "x.switch"]),
            "profiles/x.switch"
        );
        assert_eq!(
            join_paths(&["/a/b", "profiles", "{name}.switch"]),
            "/a/b/profiles/{name}.switch"
        );
        assert_eq!(join_paths(&["", "b"]), "b");
        assert_eq!(join_paths(&[]), "");
    }

    #[test]
    fn folder_and_existence_checks() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f.txt");
        std::fs::write(&file, b"x").unwrap();

        assert!(file_or_dir_exists(dir.path().to_str().unwrap()));
        assert!(file_or_dir_exists(file.to_str().unwrap()));
        assert!(!file_or_dir_exists(
            dir.path().join("missing").to_str().unwrap()
        ));

        assert!(is_folder(dir.path().to_str().unwrap()));
        assert!(!is_folder(file.to_str().unwrap()));
        assert!(!is_folder(dir.path().join("missing").to_str().unwrap()));
    }
}
