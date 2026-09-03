use std::path::Path;

use crate::config::AppConfig;
use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::outln;
use crate::paths::{
    base, default_backup_pattern, expand_path, is_folder, resolve_switch_pattern, title_case,
};
use crate::prompt::{prompt_choice, prompt_string, prompt_yes_no};
use crate::switcher::{validate_app_name, validate_profile_name, Switcher};
use crate::templates::{detect_applications, AppTemplate};

/// The interactive setup flow behind `switch add`. Port of `RunWizard`.
///
/// With nothing configured it walks first-time setup; otherwise it offers to
/// add a profile to a known app, detect a new one, or set one up by hand.
pub fn run_wizard(s: &mut Switcher, ctx: &mut Ctx) -> Result<()> {
    if s.config.apps.is_empty() {
        initial_wizard(s, ctx)
    } else {
        add_profile_wizard(s, ctx)
    }
}

fn initial_wizard(s: &mut Switcher, ctx: &mut Ctx) -> Result<()> {
    outln!(
        ctx,
        "\n┌─ Switch Setup Wizard ─────────────────────────────────────┐"
    );
    outln!(
        ctx,
        "│ 🚀 Welcome to Switch! Let's set up your first profile.   │"
    );
    outln!(
        ctx,
        "└───────────────────────────────────────────────────────────┘"
    );
    outln!(ctx);

    let detected = detect_applications(&s.home);
    let keys: Vec<String> = detected.keys().cloned().collect();

    let mut options: Vec<String> = keys
        .iter()
        .map(|k| describe(&s.home, k, &detected[k]))
        .collect();
    options.push("Other (manual setup)".to_string());

    let Some(idx) = prompt_choice(ctx, "Available applications:", &options)? else {
        return Err(Error::Cancelled);
    };

    let (app_name, auth_path, pattern) = if idx == options.len() - 1 {
        manual_details(ctx)?
    } else {
        let key = &keys[idx];
        template_details(ctx, key, &detected[key])?
    };

    // Only this branch normalises the name; the later flows take it as typed.
    let app_name = app_name.trim().to_lowercase();
    let auth_path = expand_path(&s.home, auth_path.trim());

    let profile = prompt_string(ctx, "Current profile/account name", "")?;
    let profile = profile.trim().to_string();

    confirm_and_save(s, ctx, &app_name, &profile, &auth_path, &pattern)
}

fn add_profile_wizard(s: &mut Switcher, ctx: &mut Ctx) -> Result<()> {
    outln!(
        ctx,
        "\n┌─ Switch Setup Wizard ─────────────────────────────────────┐"
    );
    outln!(
        ctx,
        "│ Add new profile                                           │"
    );
    outln!(
        ctx,
        "└───────────────────────────────────────────────────────────┘"
    );
    outln!(ctx);

    let existing: Vec<String> = s.config.apps.keys().cloned().collect();
    let mut options = existing.clone();
    options.push("Auto-detect new application".to_string());
    options.push("Manual setup".to_string());

    let Some(idx) = prompt_choice(ctx, "Choose target:", &options)? else {
        return Err(Error::Cancelled);
    };

    if idx < existing.len() {
        return add_to_existing(s, ctx, &existing[idx]);
    }
    if idx == existing.len() {
        return auto_detect(s, ctx);
    }

    let (app_name, auth_path, pattern) = manual_details(ctx)?;
    let profile = prompt_string(ctx, "Current profile name", "")?;
    let auth_path = expand_path(&s.home, &auth_path);
    confirm_and_save(s, ctx, &app_name, &profile, &auth_path, &pattern)
}

fn add_to_existing(s: &mut Switcher, ctx: &mut Ctx, app_name: &str) -> Result<()> {
    let profile = prompt_string(ctx, "New profile name", "")?;

    validate_profile_name(&profile)?;

    let app_cfg = s.config.apps[app_name].clone();
    let auth_path = expand_path(&s.home, &app_cfg.auth_path);
    let backup = resolve_switch_pattern(&app_cfg.switch_pattern, &auth_path, &profile, &s.home);
    print_summary(ctx, app_name, &profile, &auth_path, &backup);

    if !prompt_yes_no(ctx, "Save this configuration?", true)? {
        return Err(Error::Cancelled);
    }
    s.add_account(ctx, app_name, &profile)
}

fn auto_detect(s: &mut Switcher, ctx: &mut Ctx) -> Result<()> {
    let detected = detect_applications(&s.home);
    let keys: Vec<String> = detected
        .keys()
        .filter(|name| !s.config.apps.contains_key(*name))
        .cloned()
        .collect();

    if keys.is_empty() {
        outln!(ctx, "No new applications detected.");
        return Ok(());
    }

    let options: Vec<String> = keys
        .iter()
        .map(|k| describe(&s.home, k, &detected[k]))
        .collect();

    let Some(idx) = prompt_choice(ctx, "Detected applications:", &options)? else {
        return Err(Error::Cancelled);
    };

    let key = &keys[idx];
    let (app_name, auth_path, pattern) = template_details(ctx, key, &detected[key])?;
    let profile = prompt_string(ctx, "Current profile name", "")?;
    let auth_path = expand_path(&s.home, &auth_path);
    confirm_and_save(s, ctx, &app_name, &profile, &auth_path, &pattern)
}

/// Prompts for a hand-configured application.
fn manual_details(ctx: &mut Ctx) -> Result<(String, String, String)> {
    let app_name = prompt_string(ctx, "Application name", "")?;
    validate_app_name(&app_name)?;
    let auth_path = prompt_string(ctx, "Config file/folder path", "")?;
    let default = default_pattern_for(app_name.trim(), &auth_path);
    let pattern = prompt_string(ctx, "Switch pattern", &default)?;
    Ok((app_name, auth_path, pattern))
}

/// Prompts for an application matched to a built-in template.
fn template_details(
    ctx: &mut Ctx,
    key: &str,
    tpl: &AppTemplate,
) -> Result<(String, String, String)> {
    let app_name = prompt_string(ctx, "Application name", key)?;
    let auth_path = prompt_string(ctx, "Config path", &tpl.auth_path)?;
    let pattern = prompt_string(ctx, "Switch pattern", &tpl.pattern)?;
    Ok((app_name, auth_path, pattern))
}

/// Guesses a backup pattern from the shape of the path.
///
/// A final element containing a dot is taken to be a file, and its backups sit
/// beside it as `<path>.<profile>.switch`. Anything else is treated as a folder
/// and goes to `~/.switch/profiles/<app>/`, which keeps backups out of the
/// directory being backed up — nesting them there makes the copy walk its own
/// output and makes a restore delete the sibling profiles.
///
/// The dot test is textual, so a directory named `~/.confdir` is treated as a
/// file. That is harmless: its backups become siblings, not children.
fn default_pattern_for(app_name: &str, auth_path: &str) -> String {
    if base(auth_path).contains('.') {
        "{auth_path}.{name}.switch".to_string()
    } else {
        default_backup_pattern(app_name)
    }
}

fn describe(home: &Path, name: &str, tpl: &AppTemplate) -> String {
    let path = expand_path(home, &tpl.auth_path);
    let kind = if is_folder(&path) { "Folder" } else { "File" };
    format!("{}      {}  [{}]", title_case(name), path, kind)
}

fn print_summary(ctx: &mut Ctx, app_name: &str, profile: &str, auth_path: &str, backup: &str) {
    outln!(ctx, "\nSummary:");
    outln!(ctx, "  App:         {app_name}");
    outln!(ctx, "  Profile:     {profile}");
    outln!(ctx, "  Config path: {auth_path}");
    outln!(ctx, "  Backup path: {backup}");
}

fn confirm_and_save(
    s: &mut Switcher,
    ctx: &mut Ctx,
    app_name: &str,
    profile: &str,
    auth_path: &str,
    pattern: &str,
) -> Result<()> {
    validate_app_name(app_name)?;
    validate_profile_name(profile)?;

    let backup = resolve_switch_pattern(pattern, auth_path, profile, &s.home);
    print_summary(ctx, app_name, profile, auth_path, &backup);

    if !prompt_yes_no(ctx, "Save this configuration?", true)? {
        return Err(Error::Cancelled);
    }

    s.set_app_config(
        app_name,
        AppConfig {
            current: profile.to_string(),
            accounts: Vec::new(),
            auth_path: auth_path.to_string(),
            switch_pattern: pattern.to_string(),
        },
    );
    s.add_account(ctx, app_name, profile)?;

    if s.config.default.config.is_empty() {
        s.config.default.config = app_name.to_string();
        s.save()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctx::test_ctx;
    use crate::switcher::fixtures::{codex_config, setup_codex_files, switch_backup};

    fn switcher(home: &Path) -> Switcher {
        Switcher::with_home(home).unwrap()
    }

    /// Joins scripted answers into the stdin the wizard will consume.
    fn inputs(lines: &[&str]) -> String {
        format!("{}\n", lines.join("\n"))
    }

    // Port of TestRunWizard_ManualSetup_Success.
    #[test]
    fn run_wizard_manual_setup_success() {
        let home = tempfile::tempdir().unwrap();
        let auth_dir = home.path().join(".myapp");
        std::fs::create_dir_all(&auth_dir).unwrap();
        let auth_path = auth_dir.join("cfg.json");
        std::fs::write(&auth_path, br#"{"k":1}"#).unwrap();

        let stdin = inputs(&[
            "1",                         // Other (manual setup)
            "myapp",                     // app name
            auth_path.to_str().unwrap(), // config path
            "",                          // accept the default pattern
            "acc1",                      // current profile
            "",                          // accept save
            "",
        ]);
        let (mut ctx, _, _) = test_ctx(home.path(), &stdin);
        let mut s = switcher(home.path());

        run_wizard(&mut s, &mut ctx).unwrap();

        let app = s.get_app_config("myapp").expect("app not saved");
        assert_eq!(app.current, "acc1");

        let backup = resolve_switch_pattern(
            &app.switch_pattern,
            auth_path.to_str().unwrap(),
            "acc1",
            home.path(),
        );
        assert!(
            Path::new(&backup).exists(),
            "backup not created at {backup}"
        );
    }

    // Port of TestRunWizard_AddToExisting_Success.
    #[test]
    fn run_wizard_add_to_existing_success() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = setup_codex_files(
            home.path(),
            r#"{"token":"u1"}"#,
            &[("u1", r#"{"token":"u1"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("u1", &["u1"]));
        s.save().unwrap();

        let stdin = inputs(&["1", "u2", "", ""]);
        let (mut ctx, _, _) = test_ctx(home.path(), &stdin);

        run_wizard(&mut s, &mut ctx).unwrap();

        assert!(
            switch_backup(&auth_path, "u2").exists(),
            "switch backup u2 missing"
        );
        let app = s.get_app_config("codex").unwrap();
        assert!(
            app.accounts.contains(&"u2".to_string()),
            "account u2 not added"
        );
    }

    // Port of TestRunWizard_Initial_DetectedTemplate_Success.
    #[test]
    fn run_wizard_initial_detected_template_success() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"t":1}"#, &[]);

        let stdin = inputs(&[
            "1",  // the detected codex entry
            "",   // default app name
            "",   // default config path
            "",   // default switch pattern
            "p1", // profile name
            "",   // save
        ]);
        let (mut ctx, _, _) = test_ctx(home.path(), &stdin);
        let mut s = switcher(home.path());

        run_wizard(&mut s, &mut ctx).unwrap();
        assert!(s.get_app_config("codex").is_some(), "codex app not created");
    }

    // Port of TestRunWizard_Existing_AddExisting_Cancel.
    #[test]
    fn run_wizard_existing_add_existing_cancel() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"token":"u1"}"#, &[("u1", "{}")]);
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("u1", &["u1"]));
        s.save().unwrap();

        let stdin = inputs(&["1", "p", "no"]);
        let (mut ctx, _, _) = test_ctx(home.path(), &stdin);

        let err = run_wizard(&mut s, &mut ctx).unwrap_err();
        assert!(
            err.to_string().contains("cancelled"),
            "expected cancelled, got {err}"
        );
    }

    // Port of TestRunWizard_Existing_AutoDetect_NoNew.
    #[test]
    fn run_wizard_existing_auto_detect_no_new() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        // An app exists, but nothing on disk is detectable.
        s.set_app_config("codex", codex_config("a", &["a"]));
        s.save().unwrap();

        let (mut ctx, out, _) = test_ctx(home.path(), "2\n");
        run_wizard(&mut s, &mut ctx).unwrap();
        assert!(out.contents().contains("No new applications detected."));
    }

    // Port of TestRunWizard_Existing_ManualSetup_Success.
    #[test]
    fn run_wizard_existing_manual_setup_success() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a"]));
        s.save().unwrap();

        let app_dir = home.path().join(".xapp");
        std::fs::create_dir_all(&app_dir).unwrap();
        let cfg = app_dir.join("cfg.json");
        std::fs::write(&cfg, b"{}").unwrap();

        let stdin = inputs(&[
            "3",                   // Manual setup
            "xapp",                // application name
            cfg.to_str().unwrap(), // config file path
            "",                    // default pattern
            "p0",                  // current profile
            "",                    // save
        ]);
        let (mut ctx, _, _) = test_ctx(home.path(), &stdin);

        run_wizard(&mut s, &mut ctx).unwrap();
        assert!(s.get_app_config("xapp").is_some(), "xapp not created");
    }

    // Port of TestRunWizard_AutoDetect_NewApp.
    #[test]
    fn run_wizard_auto_detect_new_app() {
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".gitconfig"), b"[user]\nname = t").unwrap();

        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("c", &["c"]));
        s.save().unwrap();

        let stdin = inputs(&[
            "2",  // Auto-detect new application
            "1",  // the first detected app (git)
            "",   // default application name
            "",   // default config path
            "",   // default switch pattern
            "p1", // current profile
            "",   // save
        ]);
        let (mut ctx, _, _) = test_ctx(home.path(), &stdin);

        run_wizard(&mut s, &mut ctx).unwrap();
        assert!(
            s.get_app_config("git").is_some(),
            "git app not created by wizard"
        );
    }

    // Port of TestRunWizard_ManualFolder_DefaultPattern.
    #[test]
    fn run_wizard_manual_folder_default_pattern() {
        let home = tempfile::tempdir().unwrap();
        let conf_dir = home.path().join(".confdir");
        std::fs::create_dir_all(&conf_dir).unwrap();
        std::fs::write(conf_dir.join("a.txt"), b"data").unwrap();

        let stdin = inputs(&[
            "1",                        // Other (manual setup)
            "folderapp",                // app name
            conf_dir.to_str().unwrap(), // folder path
            "",                         // accept the default pattern
            "p1",                       // current profile
            "",                         // save
            "",
        ]);
        let (mut ctx, _, _) = test_ctx(home.path(), &stdin);
        let mut s = switcher(home.path());

        run_wizard(&mut s, &mut ctx).unwrap();

        let app = s
            .get_app_config("folderapp")
            .expect("folderapp missing from config");
        let backup = resolve_switch_pattern(
            &app.switch_pattern,
            conf_dir.to_str().unwrap(),
            "p1",
            home.path(),
        );
        assert!(
            Path::new(&backup).join("a.txt").exists(),
            "expected the copied file in the backup dir {backup}"
        );
    }

    // Port of TestRunWizard_Cancel_NoApps.
    #[test]
    fn run_wizard_cancel_no_apps() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = test_ctx(home.path(), "\n");
        let mut s = switcher(home.path());

        let err = run_wizard(&mut s, &mut ctx).unwrap_err();
        assert!(
            err.to_string().contains("cancelled"),
            "expected cancelled, got {err}"
        );
    }

    // Port of TestRunWizard_Cancel_WithExistingApps.
    #[test]
    fn run_wizard_cancel_with_existing_apps() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "\n");
        let err = run_wizard(&mut s, &mut ctx).unwrap_err();
        assert!(
            err.to_string().contains("cancelled"),
            "expected cancelled, got {err}"
        );
    }

    #[test]
    fn default_pattern_depends_on_the_last_element() {
        // A dot anywhere in the final element means "file": backups sit beside
        // the config.
        assert_eq!(
            default_pattern_for("app", "/a/b/cfg.json"),
            "{auth_path}.{name}.switch"
        );
        // A dotfile directory counts as a file too, which keeps its backups as
        // siblings rather than children.
        assert_eq!(
            default_pattern_for("app", "/a/.confdir"),
            "{auth_path}.{name}.switch"
        );
        // A folder gets its own directory under ~/.switch, never inside itself.
        assert_eq!(
            default_pattern_for("vsclone", "/a/b/User"),
            "~/.switch/profiles/vsclone/{name}"
        );
    }
}
