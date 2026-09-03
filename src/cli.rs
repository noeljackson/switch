use crate::ctx::Ctx;
use crate::error::Error;
use crate::switcher::Switcher;
use crate::wizard::run_wizard;
use crate::{errln, outln};

/// Top-level subcommands. An application may not be called any of these:
/// dispatch checks subcommands first, so such an app could never be selected.
pub const RESERVED_APP_NAMES: &[&str] = &[
    "add", "config", "default", "help", "list", "remove", "rm", "version",
];

/// Per-app subcommands, matched by `switch <app> <word>` before the word is
/// treated as a profile. A profile with one of these names would be
/// unreachable the same way.
pub const RESERVED_PROFILE_NAMES: &[&str] = &["add", "config", "list", "remove", "rm"];

pub fn is_reserved_app_name(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    RESERVED_APP_NAMES.contains(&name.as_str()) || name.starts_with('-')
}

pub fn is_reserved_profile_name(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    RESERVED_PROFILE_NAMES.contains(&name.as_str()) || name.starts_with('-')
}

/// Runs the CLI and returns the process exit code.
///
/// `args` excludes the program name. Returning the code rather than calling
/// `exit` keeps every dispatch path testable in-process.
pub fn run(args: &[String], ctx: &mut Ctx) -> i32 {
    if args.is_empty() {
        return run_default_cycle(ctx);
    }

    // Flags and `version` are answered before the config is touched, so they
    // still work when it cannot be read.
    match args[0].as_str() {
        "-v" | "-V" | "--version" | "version" => {
            outln!(ctx, "{}", short_version());
            return 0;
        }
        "-h" | "--help" | "help" => {
            print_help(ctx);
            return 0;
        }
        other if other.starts_with('-') => {
            errln!(ctx, "unknown option: {other}");
            errln!(ctx, "Run 'switch help' for usage");
            return 1;
        }
        _ => {}
    }

    let mut s = match Switcher::new(ctx) {
        Ok(s) => s,
        Err(e) => {
            print_error(ctx, &e);
            return 1;
        }
    };

    match args[0].as_str() {
        "add" => handle_add(&mut s, ctx, &args[1..]),
        "list" => handle_list(&mut s, ctx, &args[1..]),
        "rm" | "remove" => handle_remove(&mut s, ctx, &args[1..]),
        "default" => {
            if args.len() != 2 {
                usage(ctx, "switch default <app>");
                return 1;
            }
            let result = s.set_default_app(ctx, &args[1]);
            finish(ctx, result)
        }
        "config" => {
            let result = s.open_config(ctx);
            finish(ctx, result)
        }
        app => handle_app(&mut s, ctx, app, &args[1..]),
    }
}

/// Turns a fallible operation into an exit code, printing anything that is not
/// a plain cancellation.
fn finish(ctx: &mut Ctx, result: crate::error::Result<()>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(e) => {
            if !e.is_cancelled() {
                print_error(ctx, &e);
            }
            1
        }
    }
}

/// Cycles the default application.
pub fn run_default_cycle(ctx: &mut Ctx) -> i32 {
    let palette = ctx.colors;

    let mut s = match Switcher::new(ctx) {
        Ok(s) => s,
        Err(e) => {
            print_error(ctx, &e);
            return 1;
        }
    };

    let default_app = s.config.default.config.clone();
    if default_app.is_empty() {
        errln!(
            ctx,
            "{}✗ No default application configured{}",
            palette.red,
            palette.reset
        );
        errln!(ctx, "Run 'switch add' to set up an application.");
        return 1;
    }

    let result = s.cycle_accounts(ctx, &default_app);
    finish(ctx, result)
}

pub fn handle_add(s: &mut Switcher, ctx: &mut Ctx, args: &[String]) -> i32 {
    match args.len() {
        0 => {
            let result = run_wizard(s, ctx);
            finish(ctx, result)
        }
        1 => {
            let app_name = args[0].clone();
            let profile = match crate::prompt::prompt_string(ctx, "Profile name", "") {
                Ok(p) => p,
                Err(e) => return finish(ctx, Err(e)),
            };
            let result = s.add_account(ctx, &app_name, &profile);
            finish(ctx, result)
        }
        2 => {
            let result = s.add_account(ctx, &args[0], &args[1]);
            finish(ctx, result)
        }
        _ => {
            usage(ctx, "switch add <app> <account>");
            1
        }
    }
}

pub fn handle_list(s: &mut Switcher, ctx: &mut Ctx, args: &[String]) -> i32 {
    if args.is_empty() {
        s.list_all_apps(ctx);
    } else {
        s.list_accounts(ctx, &args[0]);
    }
    0
}

/// `switch rm <app>` drops an application; `switch rm <app> <profile>` drops
/// one profile. Both ask before deleting anything.
pub fn handle_remove(s: &mut Switcher, ctx: &mut Ctx, args: &[String]) -> i32 {
    match args.len() {
        1 => {
            let result = s.remove_app(ctx, &args[0]);
            finish(ctx, result)
        }
        2 => {
            let result = s.remove_account(ctx, &args[0], &args[1]);
            finish(ctx, result)
        }
        _ => {
            usage(ctx, "switch rm <app> [profile]");
            1
        }
    }
}

/// Everything that is not a known subcommand is treated as an application name.
pub fn handle_app(s: &mut Switcher, ctx: &mut Ctx, app_name: &str, args: &[String]) -> i32 {
    let palette = ctx.colors;

    match args.len() {
        0 => {
            let result = s.cycle_accounts(ctx, app_name);
            finish(ctx, result)
        }
        1 => {
            let sub = args[0].as_str();
            match sub {
                "add" => {
                    usage(ctx, "switch add <app> <account>");
                    1
                }
                "rm" | "remove" => {
                    let line = format!("switch {app_name} rm <profile>");
                    usage(ctx, &line);
                    1
                }
                "list" => {
                    s.list_accounts(ctx, app_name);
                    0
                }
                "config" => {
                    let result = s.open_config(ctx);
                    finish(ctx, result)
                }
                _ => {
                    let result = s.switch_account(ctx, app_name, sub);
                    finish(ctx, result)
                }
            }
        }
        2 if args[0] == "add" => {
            let result = s.add_account(ctx, app_name, &args[1]);
            finish(ctx, result)
        }
        2 if args[0] == "rm" || args[0] == "remove" => {
            let result = s.remove_account(ctx, app_name, &args[1]);
            finish(ctx, result)
        }
        _ => {
            errln!(
                ctx,
                "{}✗ Unknown command format{}",
                palette.red,
                palette.reset
            );
            errln!(ctx, "Run 'switch help' for usage");
            1
        }
    }
}

/// Usage hints accompany a non-zero exit, so they belong on stderr.
fn usage(ctx: &mut Ctx, line: &str) {
    errln!(ctx, "Usage: {line}");
}

pub fn print_error(ctx: &mut Ctx, err: &Error) {
    let palette = ctx.colors;
    errln!(ctx, "{}✗ Error: {err}{}", palette.red, palette.reset);
}

pub fn print_help(ctx: &mut Ctx) {
    let palette = ctx.colors;
    outln!(
        ctx,
        "{}Switch - Universal Account Switcher{}\n",
        palette.cyan,
        palette.reset
    );
    outln!(ctx, "Usage:");
    outln!(
        ctx,
        "  switch                       Cycle through default app accounts"
    );
    outln!(
        ctx,
        "  switch <app>                 Cycle through app accounts"
    );
    outln!(
        ctx,
        "  switch <app> <account>       Switch to specific account"
    );
    outln!(ctx, "  switch add                   Launch setup wizard");
    outln!(ctx, "  switch add <app>             Add a profile to app");
    outln!(
        ctx,
        "  switch add <app> <account>   Add current config as account"
    );
    outln!(
        ctx,
        "  switch rm <app> <account>    Remove a profile and its backup"
    );
    outln!(
        ctx,
        "  switch rm <app>              Remove an app and all its backups"
    );
    outln!(
        ctx,
        "  switch list                  List all apps and profiles"
    );
    outln!(
        ctx,
        "  switch list <app>            List profiles for specific app"
    );
    outln!(ctx, "  switch default <app>         Set default app");
    outln!(
        ctx,
        "  switch config                Open config file in editor"
    );
    outln!(
        ctx,
        "  switch <app> config          Open config file in editor"
    );
    outln!(
        ctx,
        "  switch -v, --version         Print short version (commit)"
    );
    outln!(ctx, "  switch -h, --help            Show this help\n");
    outln!(
        ctx,
        "Built-in templates: codex, claude, claudecode, antigravity, vscode, cursor, ssh, git"
    );
    outln!(
        ctx,
        "Colour follows NO_COLOR and whether output is a terminal."
    );
}

/// The build version, trimmed for display.
///
/// `git describe` output like `v1.2.3-4-gabc1234-dirty` reduces to the commit
/// hash; a plain tag or version is returned unchanged.
pub fn short_version() -> String {
    short_version_of(env!("SWITCH_VERSION"))
}

fn short_version_of(version: &str) -> String {
    let version = version.strip_suffix("-dirty").unwrap_or(version);

    if let Some(i) = version.rfind("-g") {
        if i + 2 < version.len() {
            return version[i + 2..].to_string();
        }
    }
    version.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::{failing_stdin_ctx, test_ctx};
    use crate::switcher::fixtures::{codex_config, setup_codex_files};
    use std::path::Path;

    fn switcher(home: &Path) -> Switcher {
        Switcher::with_home(home).unwrap()
    }

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // Port of TestShortVersion.
    #[test]
    fn short_version_trims_git_describe_output() {
        assert_eq!(short_version_of("v1.2.3-4-gabcdef"), "abcdef");
        assert_eq!(short_version_of("v1.2.3-dirty"), "v1.2.3");
        // A plain version or bare commit passes through untouched.
        assert_eq!(short_version_of("1.0.2"), "1.0.2");
        assert_eq!(short_version_of("b024d34"), "b024d34");
        // A trailing "-g" is not a commit prefix, so it stays.
        assert_eq!(short_version_of("v1.2.3-g"), "v1.2.3-g");
    }

    // Port of TestPrintHelpersAndWrappers.
    #[test]
    fn print_help_and_error_go_to_the_right_streams() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, out, err_out) = test_ctx(home.path(), "");

        print_help(&mut ctx);
        print_error(&mut ctx, &Error::new("boom"));

        assert!(out
            .contents()
            .contains("Switch - Universal Account Switcher"));
        assert!(err_out.contents().contains("boom"));
        // Help is output, not a diagnostic, so it stays on stdout.
        assert!(!err_out.contents().contains("Usage:"));
    }

    // Port of TestPrintHelp_ContainsNewCommands.
    #[test]
    fn print_help_contains_new_commands() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        print_help(&mut ctx);

        let text = out.contents();
        assert!(text.contains("switch default <app>"), "{text}");
        assert!(text.contains("switch config"), "{text}");
        assert!(text.contains("switch <app> config"), "{text}");
    }

    // Port of TestHandleAdd_ZeroArgs_Cancelled.
    #[test]
    fn handle_add_zero_args_cancelled() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = test_ctx(home.path(), "\n");
        let mut s = switcher(home.path());

        assert_eq!(handle_add(&mut s, &mut ctx, &[]), 1);
    }

    // Port of TestHandleAdd_ZeroArgs_Success.
    #[test]
    fn handle_add_zero_args_success() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".mapp")).unwrap();
        let cfg = home.path().join(".mapp").join("cfg.json");
        std::fs::write(&cfg, b"{}").unwrap();

        let stdin = format!("1\nmapp\n{}\n\np0\n\n\n", cfg.to_str().unwrap());
        let (mut ctx, _, _) = test_ctx(home.path(), &stdin);
        let mut s = switcher(home.path());

        assert_eq!(handle_add(&mut s, &mut ctx, &[]), 0);
    }

    // Port of TestHandleAdd_PromptError.
    #[test]
    fn handle_add_prompt_error() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = failing_stdin_ctx(home.path());
        let mut s = switcher(home.path());

        assert_eq!(handle_add(&mut s, &mut ctx, &args(&["codex"])), 1);
    }

    // Port of TestHandleAdd_UnknownAppError.
    #[test]
    fn handle_add_unknown_app_error() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());

        assert_eq!(handle_add(&mut s, &mut ctx, &args(&["unknown", "p"])), 1);
    }

    // Port of TestHandleAdd_TooManyArgs.
    #[test]
    fn handle_add_too_many_args() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, err_out) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());

        assert_eq!(handle_add(&mut s, &mut ctx, &args(&["a", "b", "c"])), 1);
        assert!(err_out
            .contents()
            .contains("Usage: switch add <app> <account>"));
    }

    // Port of TestHandleAddAndListAndApp.
    #[test]
    fn handle_add_and_list_and_app() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"token":"u1"}"#, &[("u1", "{}")]);
        let mut s = switcher(home.path());

        // One argument prompts for the profile name.
        let (mut ctx, out, _) = test_ctx(home.path(), "bob\n");
        assert_eq!(handle_add(&mut s, &mut ctx, &args(&["codex"])), 0);
        assert!(out.contents().contains("Profile name"));

        // Two arguments need no prompt.
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(handle_add(&mut s, &mut ctx, &args(&["codex", "carol"])), 0);

        let (mut ctx, out, _) = test_ctx(home.path(), "");
        assert_eq!(handle_list(&mut s, &mut ctx, &args(&["codex"])), 0);
        assert!(out.contents().contains("Codex"));

        let (mut ctx, out, _) = test_ctx(home.path(), "");
        assert_eq!(handle_app(&mut s, &mut ctx, "codex", &args(&["list"])), 0);
        assert!(out.contents().contains("Codex"));

        // Three arguments is not a recognised shape.
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(
            handle_app(&mut s, &mut ctx, "codex", &args(&["add", "x", "y"])),
            1
        );
    }

    // Port of TestHandleApp_AddSubcommand_Success.
    #[test]
    fn handle_app_add_subcommand_success() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"token":"u1"}"#, &[("u1", "{}")]);
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("u1", &["u1"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(
            handle_app(&mut s, &mut ctx, "codex", &args(&["add", "u2"])),
            0
        );
    }

    // Port of TestHandleApp_SwitchSubcommand.
    #[test]
    fn handle_app_switch_subcommand() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"token":"a"}"#, &[("a", "{}"), ("b", "{}")]);
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(handle_app(&mut s, &mut ctx, "codex", &args(&["b"])), 0);
    }

    // Port of TestHandleApp_Branches.
    #[test]
    fn handle_app_branches() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("", &[]));

        // Cycling with no accounts is an error.
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(handle_app(&mut s, &mut ctx, "codex", &[]), 1);

        // A bare `add` prints usage.
        let (mut ctx, _, err_out) = test_ctx(home.path(), "");
        assert_eq!(handle_app(&mut s, &mut ctx, "codex", &args(&["add"])), 1);
        assert!(err_out
            .contents()
            .contains("Usage: switch add <app> <account>"));
    }

    // Port of TestHandleApp_CycleSuccess.
    #[test]
    fn handle_app_cycle_success() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"token":"a"}"#, &[("a", "{}"), ("b", "{}")]);
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(handle_app(&mut s, &mut ctx, "codex", &[]), 0);
    }

    // Port of TestHandleApp_ConfigCommand.
    #[cfg(unix)]
    #[test]
    fn handle_app_config_command() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        ctx.editor = Some("echo".to_string());

        assert_eq!(handle_app(&mut s, &mut ctx, "codex", &args(&["config"])), 0);
    }

    // Port of TestHandleApp_ConfigCommand_Error.
    #[test]
    fn handle_app_config_command_error() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        ctx.editor = Some(String::new());
        ctx.path_env = Some(String::new());

        assert_eq!(handle_app(&mut s, &mut ctx, "codex", &args(&["config"])), 1);
    }

    // Port of TestRunDefaultCycle.
    #[test]
    fn run_default_cycle_switches_to_the_next_profile() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = setup_codex_files(
            home.path(),
            r#"{"token":"u1"}"#,
            &[("u1", r#"{"token":"u1"}"#), ("u2", r#"{"token":"u2"}"#)],
        );
        let mut s = switcher(home.path());
        s.config.default.config = "codex".to_string();
        s.set_app_config("codex", codex_config("u1", &["u1", "u2"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(run_default_cycle(&mut ctx), 0);

        let live = std::fs::read_to_string(&auth_path).unwrap();
        assert!(live.contains("u2"), "expected u2, got {live}");
    }

    // Port of TestRunDefaultCycle_NoDefault.
    #[test]
    fn run_default_cycle_no_default() {
        let home = tempfile::tempdir().unwrap();
        // Exactly the config the Go test writes by hand.
        std::fs::write(
            home.path().join(".switch.toml"),
            b"[default]\nconfig=\"\"\n\n[apps]\n",
        )
        .unwrap();

        let (mut ctx, _, err_out) = test_ctx(home.path(), "");
        assert_eq!(run_default_cycle(&mut ctx), 1);
        assert!(err_out
            .contents()
            .contains("No default application configured"));
    }

    // Port of TestRunDefaultCycle_DefaultAppMissing.
    #[test]
    fn run_default_cycle_default_app_missing() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        s.config.default.config = "codex".to_string();
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(run_default_cycle(&mut ctx), 1);
    }

    // Port of TestRunDefaultCycle_NoAccountsInDefault.
    #[test]
    fn run_default_cycle_no_accounts_in_default() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), "{}", &[]);
        let mut s = switcher(home.path());
        s.config.default.config = "codex".to_string();
        s.set_app_config("codex", codex_config("", &[]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        assert_eq!(run_default_cycle(&mut ctx), 1);
    }

    #[test]
    fn run_dispatches_top_level_commands() {
        let home = tempfile::tempdir().unwrap();
        // The "a" backup matches the live config, so "a" is the current
        // profile and cycling advances to "b".
        setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#), ("b", r#"{"token":"b"}"#)],
        );
        let mut s = switcher(home.path());
        s.config.default.config = "codex".to_string();
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        s.save().unwrap();

        // help
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        assert_eq!(run(&args(&["help"]), &mut ctx), 0);
        assert!(out
            .contents()
            .contains("Switch - Universal Account Switcher"));

        // version, both spellings
        for flag in ["-v", "--version", "version"] {
            let (mut ctx, out, _) = test_ctx(home.path(), "");
            assert_eq!(run(&args(&[flag]), &mut ctx), 0);
            assert_eq!(out.contents().trim(), short_version());
        }

        // list
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        assert_eq!(run(&args(&["list"]), &mut ctx), 0);
        assert!(out.contents().contains("Configured applications:"));

        // default <app>
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        assert_eq!(run(&args(&["default", "codex"]), &mut ctx), 0);
        assert!(out.contents().contains("Default app"));

        // default with the wrong argument count
        let (mut ctx, _, err_out) = test_ctx(home.path(), "");
        assert_eq!(run(&args(&["default"]), &mut ctx), 1);
        assert!(err_out.contents().contains("Usage: switch default <app>"));

        // an unknown first argument is treated as an app name
        let (mut ctx, _, err_out) = test_ctx(home.path(), "");
        assert_eq!(run(&args(&["nosuchapp"]), &mut ctx), 1);
        assert!(err_out
            .contents()
            .contains("no configuration found for app 'nosuchapp'"));

        // no arguments cycles the default app
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        assert_eq!(run(&[], &mut ctx), 0);
        assert!(out.contents().contains("switched from a to b"));
    }

    #[test]
    fn run_handles_rm() {
        let home = tempfile::tempdir().unwrap();
        let auth = setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#), ("b", r#"{"token":"b"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        s.config.default.config = "codex".to_string();
        s.save().unwrap();

        // Declining leaves everything in place and still exits non-zero.
        let (mut ctx, _, err_out) = test_ctx(home.path(), "no\n");
        assert_eq!(run(&args(&["rm", "codex", "b"]), &mut ctx), 1);
        assert!(
            !err_out.contents().contains("Error"),
            "a cancellation should not print an error banner"
        );

        let (mut ctx, _, _) = test_ctx(home.path(), "yes\n");
        assert_eq!(run(&args(&["rm", "codex", "b"]), &mut ctx), 0);
        assert!(!auth.with_file_name("auth.json.b.switch").exists());

        // `switch <app> rm <profile>` is the same operation.
        let (mut ctx, _, _) = test_ctx(home.path(), "yes\n");
        assert_eq!(run(&args(&["codex", "rm", "a"]), &mut ctx), 0);
        assert!(switcher(home.path())
            .get_app_config("codex")
            .unwrap()
            .accounts
            .is_empty());

        // Wrong argument counts print usage, on stderr.
        let (mut ctx, out, err_out) = test_ctx(home.path(), "");
        assert_eq!(run(&args(&["rm"]), &mut ctx), 1);
        assert!(err_out.contents().contains("Usage: switch rm"));
        assert!(out.contents().is_empty(), "usage must not go to stdout");
    }

    #[test]
    fn run_removes_a_whole_app() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "yes\n");
        assert_eq!(run(&args(&["remove", "codex"]), &mut ctx), 0);
        assert!(switcher(home.path()).get_app_config("codex").is_none());
    }

    #[test]
    fn reserved_names_cover_every_subcommand() {
        // Anything dispatch handles before falling through to an app name must
        // be reserved, or an app of that name would be unreachable.
        for name in RESERVED_APP_NAMES {
            assert!(is_reserved_app_name(name), "{name}");
        }
        for name in [
            "add", "list", "rm", "remove", "default", "config", "help", "version",
        ] {
            assert!(
                RESERVED_APP_NAMES.contains(&name),
                "{name} is dispatched as a subcommand but is not reserved"
            );
        }
        assert!(!is_reserved_app_name("codex"));
    }

    #[test]
    fn help_lists_the_new_commands() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        print_help(&mut ctx);

        let text = out.contents();
        for expected in [
            "switch rm <app>",
            "switch default <app>",
            "-h, --help",
            "-v, --version",
        ] {
            assert!(
                text.contains(expected),
                "help should mention {expected}: {text}"
            );
        }
    }

    #[test]
    fn flags_work_without_a_readable_config() {
        let home = tempfile::tempdir().unwrap();
        // A directory where the config file belongs: anything that loads the
        // config will fail.
        std::fs::create_dir_all(home.path().join(".switch.toml")).unwrap();

        for flag in ["-h", "--help", "help"] {
            let (mut ctx, out, _) = test_ctx(home.path(), "");
            assert_eq!(run(&args(&[flag]), &mut ctx), 0, "{flag}");
            assert!(out.contents().contains("Usage:"), "{flag}");
        }

        for flag in ["-v", "-V", "--version", "version"] {
            let (mut ctx, out, _) = test_ctx(home.path(), "");
            assert_eq!(run(&args(&[flag]), &mut ctx), 0, "{flag}");
            assert_eq!(out.contents().trim(), short_version(), "{flag}");
        }
    }

    #[test]
    fn an_unknown_option_is_reported_as_one() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, err_out) = test_ctx(home.path(), "");

        assert_eq!(run(&args(&["--nope"]), &mut ctx), 1);
        let text = err_out.contents();
        assert!(text.contains("unknown option: --nope"), "{text}");
        // Not mistaken for an application name.
        assert!(!text.contains("no configuration found"), "{text}");
    }

    #[test]
    fn run_reports_an_unreadable_config() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".switch.toml")).unwrap();

        let (mut ctx, _, err_out) = test_ctx(home.path(), "");
        assert_eq!(run(&args(&["list"]), &mut ctx), 1);
        assert!(err_out.contents().contains("Error:"));
    }
}
