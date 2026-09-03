//! End-to-end tests that drive the real binary.
//!
//! The Go suite reached `main()` by re-executing its own test binary with a
//! `GO_WANT_HELPER_PROCESS` shim, because that was the only way to get at it.
//! Cargo hands us the built binary's path directly, so these spawn it for real.
//! Running as a child process is also what makes `HOME` and `EDITOR` safe to
//! set: they go to that one child, not to the whole test process.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use switch::config::{save_config, AppConfig, Config, DefaultConfig};

const BIN: &str = env!("CARGO_BIN_EXE_switch");

/// Runs the binary with `home` as the user's home directory and returns its
/// exit code plus stdout and stderr combined.
///
/// stdin is closed so a command that unexpectedly prompts gets EOF rather than
/// hanging the test run.
fn run(args: &[&str], home: &Path) -> (i32, String) {
    run_with_env(args, home, &[])
}

fn run_with_env(args: &[&str], home: &Path, extra_env: &[(&str, &str)]) -> (i32, String) {
    let mut cmd = Command::new(BIN);
    cmd.args(args)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    for (key, value) in extra_env {
        cmd.env(key, value);
    }

    let output = cmd.output().expect("failed to run the switch binary");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), combined)
}

fn codex_app() -> AppConfig {
    AppConfig {
        current: String::new(),
        accounts: Vec::new(),
        auth_path: "~/.codex/auth.json".to_string(),
        switch_pattern: "{auth_path}.{name}.switch".to_string(),
    }
}

/// Writes a `~/.switch.toml` with the given apps and default.
fn seed_config(home: &Path, default_app: &str, apps: &[(&str, AppConfig)]) {
    let config = Config {
        default: DefaultConfig {
            config: default_app.to_string(),
        },
        apps: apps
            .iter()
            .map(|(name, cfg)| (name.to_string(), cfg.clone()))
            .collect::<BTreeMap<_, _>>(),
    };
    save_config(&home.join(".switch.toml"), &config).unwrap();
}

fn seed_codex_auth(home: &Path) {
    let codex = home.join(".codex");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::write(codex.join("auth.json"), b"{}").unwrap();
}

// Port of TestMain_CLI_Subprocess.
#[test]
fn cli_help_version_and_error_paths() {
    let home = tempfile::tempdir().unwrap();

    let (code, out) = run(&["help"], home.path());
    assert_eq!(code, 0, "help exited {code}: {out}");
    assert!(out.contains("Usage:"), "{out}");

    let (code, out) = run(&["version"], home.path());
    assert_eq!(code, 0, "version exited {code}: {out}");
    assert!(!out.trim().is_empty(), "version printed nothing");

    // Cycling with no config at all cannot succeed.
    let empty = tempfile::tempdir().unwrap();
    let (code, _) = run(&[], empty.path());
    assert_ne!(code, 0, "expected non-zero for an empty default cycle");

    // An unrecognised argument shape is a usage error.
    let (code, _) = run(&["codex", "add", "x", "y"], home.path());
    assert_eq!(code, 1, "expected 1 for a bad command format");
}

// Port of TestMain_CLI_Subprocess_ListAndAdd.
#[test]
fn cli_list_and_add() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_auth(home.path());
    seed_config(home.path(), "codex", &[("codex", codex_app())]);

    let (code, out) = run(&["list"], home.path());
    assert_eq!(code, 0, "list exited {code}: {out}");

    let (code, out) = run(&["add", "codex", "bob"], home.path());
    assert_eq!(code, 0, "add exited {code}: {out}");
    assert!(
        home.path()
            .join(".codex")
            .join("auth.json.bob.switch")
            .exists(),
        "the backup should have been written"
    );
}

// Port of TestMain_CLI_Subprocess_AppCommands.
#[test]
fn cli_app_commands() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_auth(home.path());
    seed_config(home.path(), "codex", &[("codex", codex_app())]);

    let (code, out) = run(&["codex", "list"], home.path());
    assert_eq!(code, 0, "codex list exited {code}: {out}");
    assert!(out.contains("Codex"), "{out}");

    let (code, out) = run(&["codex", "add", "p1"], home.path());
    assert_eq!(code, 0, "codex add exited {code}: {out}");

    let (code, out) = run(&["codex", "p1"], home.path());
    assert_eq!(code, 0, "codex switch exited {code}: {out}");
}

// Port of TestMain_CLI_Subprocess_DefaultCommand.
#[test]
fn cli_default_command() {
    let home = tempfile::tempdir().unwrap();
    let vscode = AppConfig {
        current: String::new(),
        accounts: Vec::new(),
        auth_path: "~/.vscode/User".to_string(),
        switch_pattern: "~/.vscode/profiles/{name}.switch".to_string(),
    };
    seed_config(
        home.path(),
        "codex",
        &[("codex", codex_app()), ("vscode", vscode)],
    );

    let (code, out) = run(&["default", "vscode"], home.path());
    assert_eq!(code, 0, "default command exited {code}: {out}");

    let (code, _) = run(&["default"], home.path());
    assert_eq!(code, 1, "expected 1 for default without an app");

    let (code, _) = run(&["default", "a", "b"], home.path());
    assert_eq!(code, 1, "expected 1 for default with too many arguments");

    let (code, _) = run(&["default", "nonexistent"], home.path());
    assert_eq!(code, 1, "expected 1 for default with an unknown app");
}

// Port of TestMain_CLI_Subprocess_ConfigCommand (skipped on Windows there).
#[cfg(unix)]
#[test]
fn cli_config_command() {
    let home = tempfile::tempdir().unwrap();
    seed_config(home.path(), "codex", &[]);

    let (code, out) = run_with_env(&["config"], home.path(), &[("EDITOR", "echo")]);
    assert_eq!(code, 0, "config exited {code}: {out}");

    let (code, out) = run_with_env(&["codex", "config"], home.path(), &[("EDITOR", "echo")]);
    assert_eq!(code, 0, "app config exited {code}: {out}");

    let (code, _) = run_with_env(&["config"], home.path(), &[("EDITOR", ""), ("PATH", "")]);
    assert_eq!(code, 1, "expected 1 when no editor can be found");
}

#[test]
fn piped_output_has_no_escape_sequences() {
    let home = tempfile::tempdir().unwrap();
    seed_codex_auth(home.path());
    seed_config(home.path(), "codex", &[("codex", codex_app())]);

    // stdout is a pipe here, so colour is off.
    let (code, out) = run(&["list"], home.path());
    assert_eq!(code, 0);
    assert!(out.contains("Configured applications:"), "{out}");
    assert!(
        !out.contains('\x1b'),
        "escape sequences in piped output: {out:?}"
    );

    // CLICOLOR_FORCE brings it back for CI logs.
    let (code, out) = run_with_env(&["list"], home.path(), &[("CLICOLOR_FORCE", "1")]);
    assert_eq!(code, 0);
    assert!(
        out.contains('\x1b'),
        "CLICOLOR_FORCE should re-enable colour: {out:?}"
    );

    // NO_COLOR wins over the force flag.
    let (code, out) = run_with_env(
        &["list"],
        home.path(),
        &[("CLICOLOR_FORCE", "1"), ("NO_COLOR", "1")],
    );
    assert_eq!(code, 0);
    assert!(!out.contains('\x1b'), "NO_COLOR should win: {out:?}");
}

#[test]
fn removing_a_profile_from_the_command_line() {
    use std::io::Write;

    let home = tempfile::tempdir().unwrap();
    seed_codex_auth(home.path());
    seed_config(home.path(), "codex", &[("codex", codex_app())]);

    let (code, out) = run(&["add", "codex", "work"], home.path());
    assert_eq!(code, 0, "{out}");
    let backup = home.path().join(".codex").join("auth.json.work.switch");
    assert!(backup.exists());

    // Removal asks first, so it needs a real stdin.
    let mut child = Command::new(BIN)
        .args(["rm", "codex", "work"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"yes\n").unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    assert!(!backup.exists(), "the backup should be gone");
    assert!(
        home.path().join(".codex").join("auth.json").exists(),
        "the live config must survive a removal"
    );
}

/// The `ssh` template used to back `~/.ssh` up into itself, which ran away on
/// the second `add`. End to end, both adds should now succeed and leave the
/// backups outside `~/.ssh`.
#[test]
fn the_ssh_template_backs_up_outside_the_ssh_directory() {
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("id_rsa"), b"KEY").unwrap();
    std::fs::write(ssh.join("config"), b"cfg").unwrap();

    let (code, out) = run(&["add", "ssh", "work"], home.path());
    assert_eq!(code, 0, "first add: {out}");
    let (code, out) = run(&["add", "ssh", "home"], home.path());
    assert_eq!(code, 0, "second add: {out}");

    let store = home.path().join(".switch").join("profiles").join("ssh");
    assert!(store.join("work").join("id_rsa").exists(), "{out}");
    assert!(store.join("home").join("id_rsa").exists(), "{out}");
    assert!(
        !ssh.join("profiles").exists(),
        "nothing should be written inside ~/.ssh"
    );

    // Switching between them works and does not accumulate anything.
    let (code, out) = run(&["ssh", "home"], home.path());
    assert_eq!(code, 0, "{out}");
    let entries: Vec<_> = std::fs::read_dir(&ssh)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        entries.len(),
        2,
        "unexpected contents of ~/.ssh: {entries:?}"
    );
}

/// A hand-written `switch_pattern` that nests the backup inside the config
/// folder is refused rather than run: the copy would walk its own output, and
/// a restore would delete the sibling profiles.
#[test]
fn a_nested_backup_layout_is_refused() {
    let home = tempfile::tempdir().unwrap();
    let ssh = home.path().join(".ssh");
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("id_rsa"), b"KEY").unwrap();

    let nested = AppConfig {
        current: String::new(),
        accounts: vec!["work".to_string()],
        auth_path: "~/.ssh".to_string(),
        switch_pattern: "~/.ssh/profiles/{name}.switch".to_string(),
    };
    seed_config(home.path(), "ssh", &[("ssh", nested)]);
    let backup = ssh.join("profiles").join("work.switch");
    std::fs::create_dir_all(&backup).unwrap();
    std::fs::write(backup.join("id_rsa"), b"WORK").unwrap();

    let (code, out) = run(&["ssh", "work"], home.path());
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("is inside the config path"), "{out}");

    // Nothing was touched.
    assert_eq!(std::fs::read(ssh.join("id_rsa")).unwrap(), b"KEY");
}

/// Flags are answered without reading the config, and an unrecognised one is
/// reported as an option rather than as a missing application.
#[test]
fn flags_and_unknown_options() {
    let home = tempfile::tempdir().unwrap();

    for flag in ["-h", "--help"] {
        let (code, out) = run(&[flag], home.path());
        assert_eq!(code, 0, "{flag}: {out}");
        assert!(out.contains("Usage:"), "{flag}: {out}");
    }
    for flag in ["-v", "-V", "--version"] {
        let (code, out) = run(&[flag], home.path());
        assert_eq!(code, 0, "{flag}: {out}");
        assert!(!out.trim().is_empty(), "{flag}");
    }

    let (code, out) = run(&["--nope"], home.path());
    assert_eq!(code, 1);
    assert!(out.contains("unknown option: --nope"), "{out}");
}

/// Usage hints accompany a failure, so they belong on stderr.
#[test]
fn usage_hints_go_to_stderr() {
    let home = tempfile::tempdir().unwrap();
    seed_config(home.path(), "codex", &[("codex", codex_app())]);

    let output = Command::new(BIN)
        .args(["default"])
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stdout).is_empty(),
        "stdout should be empty"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Usage: switch default <app>"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The wizard reached through a real pipe, which the Go suite never covered:
/// its subprocess tests always closed stdin.
#[test]
fn cli_wizard_over_a_pipe() {
    use std::io::Write;

    let home = tempfile::tempdir().unwrap();
    let app_dir = home.path().join(".myapp");
    std::fs::create_dir_all(&app_dir).unwrap();
    let cfg = app_dir.join("cfg.json");
    std::fs::write(&cfg, br#"{"k":1}"#).unwrap();

    let mut child = Command::new(BIN)
        .arg("add")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn the switch binary");

    let answers = format!("1\nmyapp\n{}\n\nacc1\n\n\n", cfg.to_str().unwrap());
    child
        .stdin
        .take()
        .unwrap()
        .write_all(answers.as_bytes())
        .unwrap();

    let output = child.wait_with_output().unwrap();
    let out = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "wizard exited: {out}");
    assert!(out.contains("Added account: acc1 for myapp"), "{out}");
    assert!(
        app_dir.join("cfg.json.acc1.switch").exists(),
        "the wizard should have written the backup"
    );
}
