use std::path::{Path, PathBuf};
use std::process::Command;

use crate::cli::{is_reserved_app_name, is_reserved_profile_name};
use crate::compare::content_equal;
use crate::config::{load_config, save_config, AppConfig, Config};
use crate::ctx::Ctx;
use crate::error::{Error, Result};
use crate::fsops::{overlaps, remove_path, replace_path};
use crate::paths::{expand_path, resolve_switch_pattern, switch_dir, title_case};
use crate::templates::app_template;
use crate::{errln, out, outln};

/// Which profile the live config currently corresponds to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Current {
    /// The live config matches this profile's backup.
    Matched(String),
    /// Nothing matches, but this is the profile last switched to. The live
    /// config has been edited since.
    Drifted(String),
    /// No profile can be identified.
    None,
}

impl Current {
    pub fn name(&self) -> &str {
        match self {
            Current::Matched(n) | Current::Drifted(n) => n,
            Current::None => "",
        }
    }
}

/// Holds the loaded config and the paths needed to act on it.
///
/// `home` is carried as a value rather than read from the environment on every
/// call, which is what lets tests point an instance at a temporary directory
/// without mutating process-wide state.
pub struct Switcher {
    pub config_path: PathBuf,
    pub config: Config,
    pub home: PathBuf,
}

impl Switcher {
    /// Builds a switcher for the real user.
    pub fn new(ctx: &Ctx) -> Result<Switcher> {
        if ctx.home.as_os_str().is_empty() {
            return Err(Error::new(
                "get home dir: could not determine home directory",
            ));
        }
        Switcher::with_home(&ctx.home)
    }

    /// Builds a switcher rooted at an arbitrary home directory.
    pub fn with_home(home: impl AsRef<Path>) -> Result<Switcher> {
        let home = home.as_ref();
        Switcher::with_config_path(home.join(".switch.toml"), home)
    }

    pub fn with_config_path(
        config_path: impl Into<PathBuf>,
        home: impl AsRef<Path>,
    ) -> Result<Switcher> {
        let config_path = config_path.into();
        let config = load_config(&config_path)?;
        Ok(Switcher {
            config_path,
            config,
            home: home.as_ref().to_path_buf(),
        })
    }

    pub fn save(&self) -> Result<()> {
        save_config(&self.config_path, &self.config)
    }

    /// Returns a copy of an app's configuration.
    pub fn get_app_config(&self, app_name: &str) -> Option<AppConfig> {
        self.config.apps.get(app_name).cloned()
    }

    pub fn set_app_config(&mut self, app_name: &str, config: AppConfig) {
        self.config.apps.insert(app_name.to_string(), config);
    }

    fn expand(&self, p: &str) -> String {
        expand_path(&self.home, p)
    }

    fn switch_path(&self, app_config: &AppConfig, auth_path: &str, name: &str) -> String {
        resolve_switch_pattern(&app_config.switch_pattern, auth_path, name, &self.home)
    }

    /// Refuses layouts where the backup sits inside the config it backs up (or
    /// the other way round).
    ///
    /// Copying such a tree walks its own output, and restoring it would delete
    /// the sibling profiles. No template produces this, but a hand-edited
    /// `switch_pattern` can.
    fn check_layout(&self, auth_path: &str, switch_path: &str) -> Result<()> {
        if overlaps(auth_path, switch_path) {
            return Err(Error::new(format!(
                "backup path {switch_path} is inside the config path {auth_path}; \
                 point switch_pattern somewhere outside it"
            )));
        }
        Ok(())
    }

    /// Creates `~/.switch` with owner-only permissions before anything is
    /// written into it. Profile backups can hold credentials, so the directory
    /// listing should not be world-readable.
    fn ensure_private_store(&self, switch_path: &str) -> Result<()> {
        let store = switch_dir(&self.home);
        if !overlaps(&store, switch_path) {
            return Ok(());
        }
        std::fs::create_dir_all(&store).map_err(|e| Error::new(format!("mkdir {store}: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o700));
        }
        Ok(())
    }

    /// Saves the current config as a named profile.
    pub fn add_account(&mut self, ctx: &mut Ctx, app_name: &str, account_name: &str) -> Result<()> {
        let palette = ctx.colors;
        validate_profile_name(account_name)?;

        let mut app_config = match self.get_app_config(app_name) {
            Some(config) => config,
            None => {
                validate_app_name(app_name)?;
                let template = app_template(app_name).ok_or_else(|| {
                    Error::new(format!("no configuration found for app '{app_name}'"))
                })?;

                let auth_path = self.expand(&template.auth_path);
                if std::fs::metadata(&auth_path).is_err() {
                    return Err(Error::new(format!("auth path not found: {auth_path}")));
                }

                AppConfig {
                    current: String::new(),
                    accounts: Vec::new(),
                    auth_path: template.auth_path,
                    switch_pattern: template.pattern,
                }
            }
        };

        let auth_path = self.expand(&app_config.auth_path);
        let switch_path = self.switch_path(&app_config, &auth_path, account_name);
        self.check_layout(&auth_path, &switch_path)?;

        if app_config.accounts.iter().any(|a| a == account_name) {
            outln!(
                ctx,
                "{}✗ Account '{account_name}' already exists for {app_name}{}",
                palette.red,
                palette.reset
            );
            out!(ctx, "Overwrite? (yes/no): ");
            // A read failure leaves an empty response, which is not consent.
            let response = ctx.read_line().unwrap_or_default();
            let response = response.trim().to_lowercase();
            if response != "yes" && response != "y" {
                outln!(ctx, "{}Cancelled{}", palette.yellow, palette.reset);
                return Err(Error::Cancelled);
            }
        }

        self.ensure_private_store(&switch_path)?;
        // Replace rather than merge, so re-adding a profile does not leave
        // stale files from the previous capture in the backup.
        replace_path(&auth_path, &switch_path)
            .map_err(|e| Error::new(format!("copy config: {e}")))?;

        if !app_config.accounts.iter().any(|a| a == account_name) {
            app_config.accounts.push(account_name.to_string());
            app_config.accounts.sort();
        }
        if app_config.current.is_empty() {
            app_config.current = account_name.to_string();
        }

        self.set_app_config(app_name, app_config);
        if let Err(e) = self.save() {
            // Roll the backup back out so a failed save leaves no orphan.
            let _ = remove_path(&switch_path);
            return Err(e);
        }

        outln!(
            ctx,
            "{}✓ Added account: {account_name} for {app_name}{}",
            palette.green,
            palette.reset
        );
        Ok(())
    }

    /// Makes a named profile the live config.
    pub fn switch_account(
        &mut self,
        ctx: &mut Ctx,
        app_name: &str,
        account_name: &str,
    ) -> Result<()> {
        let palette = ctx.colors;

        let mut app_config = self
            .get_app_config(app_name)
            .ok_or_else(|| Error::new(format!("no configuration found for app '{app_name}'")))?;

        if account_name.is_empty() {
            return self.cycle_accounts(ctx, app_name);
        }

        if !app_config.accounts.iter().any(|a| a == account_name) {
            return Err(Error::new(format!(
                "account '{account_name}' not found for {app_name}"
            )));
        }

        let auth_path = self.expand(&app_config.auth_path);
        let switch_path = self.switch_path(&app_config, &auth_path, account_name);
        self.check_layout(&auth_path, &switch_path)?;

        if std::fs::metadata(&switch_path).is_err() {
            return Err(Error::new(format!("switch file not found: {switch_path}")));
        }

        let current = self.current_profile(app_name);
        let current_account = current.name().to_string();

        if !current_account.is_empty() && current_account != account_name {
            // Save whatever is live under the profile it belongs to. Go ignored
            // a failure here; losing the live config is worse than a failed
            // command, so this stops the switch instead.
            let current_switch_path = self.switch_path(&app_config, &auth_path, &current_account);
            self.ensure_private_store(&current_switch_path)?;
            replace_path(&auth_path, &current_switch_path).map_err(|e| {
                Error::new(format!(
                    "could not back up the current profile '{current_account}', \
                     so the switch was stopped to avoid losing it: {e}"
                ))
            })?;
        }

        // Replace rather than merge: a profile without a credentials file must
        // not inherit the outgoing profile's.
        replace_path(&switch_path, &auth_path)
            .map_err(|e| Error::new(format!("switch config: {e}")))?;

        app_config.current = account_name.to_string();
        self.set_app_config(app_name, app_config);
        if let Err(e) = self.save() {
            // The files have already been swapped, so this is a warning rather
            // than a failure; the profile is still detected from its contents.
            errln!(
                ctx,
                "{}⚠ Warning: switched, but the config file could not be saved: {e}{}",
                palette.yellow,
                palette.reset
            );
        }

        if !current_account.is_empty() && current_account != account_name {
            outln!(
                ctx,
                "{}✓ {} account switched from {current_account} to {account_name}!{}",
                palette.green,
                title_case(app_name),
                palette.reset
            );
        } else {
            outln!(
                ctx,
                "{}✓ Switched to: {account_name}{}",
                palette.green,
                palette.reset
            );
        }
        Ok(())
    }

    /// Moves to the next profile in the list.
    pub fn cycle_accounts(&mut self, ctx: &mut Ctx, app_name: &str) -> Result<()> {
        let palette = ctx.colors;

        let app_config = self
            .get_app_config(app_name)
            .ok_or_else(|| Error::new(format!("no configuration found for app '{app_name}'")))?;

        if app_config.accounts.is_empty() {
            // Accompanies a non-zero exit, so it belongs on stderr.
            errln!(
                ctx,
                "{}✗ No accounts configured for {app_name}{}",
                palette.red,
                palette.reset
            );
            errln!(
                ctx,
                "Run 'switch add {app_name} <name>' to add your first account"
            );
            return Err(Error::new("no accounts"));
        }

        let current = self.find_current_account(app_name);
        let next = if current.is_empty() {
            app_config.accounts[0].clone()
        } else {
            app_config
                .accounts
                .iter()
                .position(|a| *a == current)
                .map(|i| app_config.accounts[(i + 1) % app_config.accounts.len()].clone())
                .unwrap_or_else(|| app_config.accounts[0].clone())
        };

        self.switch_account(ctx, app_name, &next)
    }

    /// Removes a profile and its backup.
    pub fn remove_account(
        &mut self,
        ctx: &mut Ctx,
        app_name: &str,
        account_name: &str,
    ) -> Result<()> {
        let palette = ctx.colors;

        let mut app_config = self
            .get_app_config(app_name)
            .ok_or_else(|| Error::new(format!("no configuration found for app '{app_name}'")))?;

        if !app_config.accounts.iter().any(|a| a == account_name) {
            return Err(Error::new(format!(
                "account '{account_name}' not found for {app_name}"
            )));
        }

        let auth_path = self.expand(&app_config.auth_path);
        let switch_path = self.switch_path(&app_config, &auth_path, account_name);

        // Removing the profile you are on would leave the live config
        // unbacked, so say so before asking.
        if self.find_current_account(app_name) == account_name {
            outln!(
                ctx,
                "{}! '{account_name}' is the profile currently in use; the live config at \
                 {auth_path} is left untouched.{}",
                palette.yellow,
                palette.reset
            );
        }

        out!(
            ctx,
            "Remove profile '{account_name}' from {app_name}? (yes/no): "
        );
        let response = ctx.read_line().unwrap_or_default();
        if !is_yes(&response) {
            outln!(ctx, "{}Cancelled{}", palette.yellow, palette.reset);
            return Err(Error::Cancelled);
        }

        remove_path(&switch_path).map_err(|e| Error::new(format!("remove backup: {e}")))?;

        app_config.accounts.retain(|a| a != account_name);
        if app_config.current == account_name {
            app_config.current = String::new();
        }
        self.set_app_config(app_name, app_config);
        self.save()?;

        outln!(
            ctx,
            "{}✓ Removed profile: {account_name} from {app_name}{}",
            palette.green,
            palette.reset
        );
        Ok(())
    }

    /// Removes an application entirely, along with every profile backup.
    pub fn remove_app(&mut self, ctx: &mut Ctx, app_name: &str) -> Result<()> {
        let palette = ctx.colors;

        let app_config = self
            .get_app_config(app_name)
            .ok_or_else(|| Error::new(format!("no configuration found for app '{app_name}'")))?;

        let auth_path = self.expand(&app_config.auth_path);
        let count = app_config.accounts.len();

        out!(
            ctx,
            "Remove {app_name} and its {count} profile backup(s)? \
             The live config at {auth_path} is left untouched. (yes/no): "
        );
        let response = ctx.read_line().unwrap_or_default();
        if !is_yes(&response) {
            outln!(ctx, "{}Cancelled{}", palette.yellow, palette.reset);
            return Err(Error::Cancelled);
        }

        for account in &app_config.accounts {
            let switch_path = self.switch_path(&app_config, &auth_path, account);
            remove_path(&switch_path)
                .map_err(|e| Error::new(format!("remove backup {switch_path}: {e}")))?;
        }

        self.config.apps.remove(app_name);
        if self.config.default.config == app_name {
            self.config.default.config =
                self.config.apps.keys().next().cloned().unwrap_or_default();
        }
        self.save()?;

        outln!(
            ctx,
            "{}✓ Removed application: {app_name}{}",
            palette.green,
            palette.reset
        );
        Ok(())
    }

    /// Works out which profile the live config corresponds to.
    ///
    /// Contents win: comparing against each backup means a config edited by
    /// hand, or by the application itself, is still identified correctly. The
    /// profile recorded in the config file is checked first (it is nearly
    /// always the answer) and is used as a fallback when nothing matches, so a
    /// drifted config still reports the profile it was switched to.
    pub fn current_profile(&self, app_name: &str) -> Current {
        let Some(app_config) = self.get_app_config(app_name) else {
            return Current::None;
        };

        let auth_path = self.expand(&app_config.auth_path);
        if std::fs::metadata(&auth_path).is_err() {
            return Current::None;
        }

        let recorded = app_config.current.clone();
        let recorded_is_real = !recorded.is_empty() && app_config.accounts.contains(&recorded);

        let ordered = recorded_is_real
            .then(|| recorded.clone())
            .into_iter()
            .chain(
                app_config
                    .accounts
                    .iter()
                    .filter(|n| **n != recorded)
                    .cloned(),
            );

        for account_name in ordered {
            let switch_path = self.switch_path(&app_config, &auth_path, &account_name);
            if std::fs::metadata(&switch_path).is_err() {
                continue;
            }
            if content_equal(&auth_path, &switch_path) {
                return Current::Matched(account_name);
            }
        }

        if recorded_is_real {
            Current::Drifted(recorded)
        } else {
            Current::None
        }
    }

    /// The name of the current profile, or an empty string.
    pub fn find_current_account(&self, app_name: &str) -> String {
        self.current_profile(app_name).name().to_string()
    }

    pub fn list_accounts(&self, ctx: &mut Ctx, app_name: &str) {
        let palette = ctx.colors;

        if app_name.is_empty() {
            self.list_all_apps(ctx);
            return;
        }

        let Some(app_config) = self.get_app_config(app_name) else {
            outln!(
                ctx,
                "{}✗ No accounts configured for {app_name}{}",
                palette.red,
                palette.reset
            );
            outln!(
                ctx,
                "Run 'switch add {app_name} <name>' to add your first account"
            );
            return;
        };

        let current = self.current_profile(app_name);
        outln!(
            ctx,
            "{}{} accounts:{}",
            palette.cyan,
            title_case(app_name),
            palette.reset
        );
        for acc in &app_config.accounts {
            if *acc == current.name() {
                let label = match current {
                    // Say so when the live config no longer matches the backup,
                    // rather than claiming they are the same.
                    Current::Drifted(_) => "(current, modified)",
                    _ => "(current)",
                };
                outln!(
                    ctx,
                    "  {}●{} {acc} {}{label}{}",
                    palette.green,
                    palette.reset,
                    palette.yellow,
                    palette.reset
                );
            } else {
                outln!(ctx, "  ○ {acc}");
            }
        }
    }

    pub fn list_all_apps(&self, ctx: &mut Ctx) {
        let palette = ctx.colors;

        if self.config.apps.is_empty() {
            outln!(
                ctx,
                "{}✗ No applications configured{}",
                palette.red,
                palette.reset
            );
            outln!(ctx, "Run 'switch add' to set up your first application");
            return;
        }

        outln!(
            ctx,
            "{}Configured applications:{}",
            palette.cyan,
            palette.reset
        );
        for (app_name, app_config) in &self.config.apps {
            let current = self.current_profile(app_name);
            let account_count = app_config.accounts.len();

            if *app_name == self.config.default.config {
                out!(
                    ctx,
                    "  {}●{} {app_name} ({account_count} accounts) {}(default){}",
                    palette.green,
                    palette.reset,
                    palette.yellow,
                    palette.reset
                );
            } else {
                out!(ctx, "  ○ {app_name} ({account_count} accounts)");
            }

            match &current {
                Current::Matched(name) => out!(ctx, " - current: {name}"),
                Current::Drifted(name) => out!(ctx, " - current: {name} (modified)"),
                Current::None => {}
            }
            outln!(ctx);
        }
    }

    pub fn set_default_app(&mut self, ctx: &mut Ctx, app_name: &str) -> Result<()> {
        let palette = ctx.colors;

        if self.get_app_config(app_name).is_none() {
            return Err(Error::new(format!("app '{app_name}' not found")));
        }

        let old_default = self.config.default.config.clone();
        self.config.default.config = app_name.to_string();
        self.save()
            .map_err(|e| Error::new(format!("save config: {e}")))?;

        if !old_default.is_empty() {
            outln!(
                ctx,
                "{}✓ Default app changed from {old_default} to {app_name}{}",
                palette.green,
                palette.reset
            );
        } else {
            outln!(
                ctx,
                "{}✓ Default app set to {app_name}{}",
                palette.green,
                palette.reset
            );
        }
        Ok(())
    }

    /// Opens the config file in the user's editor.
    pub fn open_config(&self, ctx: &Ctx) -> Result<()> {
        self.open_config_with(ctx.editor.as_deref(), ctx.path_env.as_deref())
    }

    /// The editor lookup with `EDITOR` and `PATH` supplied explicitly.
    ///
    /// Splitting this out is what lets the editor tests run without touching
    /// process-wide environment variables. `None` means "read from the
    /// environment"; `Some("")` means "explicitly empty".
    pub fn open_config_with(&self, editor: Option<&str>, path_env: Option<&str>) -> Result<()> {
        let mut program = editor.unwrap_or_default().to_string();

        if program.is_empty() {
            for candidate in ["nano", "vi", "vim", "code", "gedit"] {
                if let Some(found) = look_path(candidate, path_env) {
                    program = found.to_string_lossy().into_owned();
                    break;
                }
            }
        }

        if program.is_empty() {
            return Err(Error::new(
                "no text editor found. Set EDITOR environment variable or install nano/vim/code",
            ));
        }

        let status = Command::new(&program)
            .arg(&self.config_path)
            .status()
            .map_err(|e| Error::new(format!("{program}: {e}")))?;

        if !status.success() {
            return Err(Error::new(format!("{program}: exited with {status}")));
        }
        Ok(())
    }
}

fn is_yes(response: &str) -> bool {
    let response = response.trim().to_lowercase();
    response == "yes" || response == "y"
}

/// Rejects application names that cannot be used.
///
/// A name matching a subcommand is accepted by the config but unreachable:
/// `switch list` always lists applications, so an app called `list` could never
/// be selected. Separators would also send backups to unintended paths.
pub fn validate_app_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::new("application name cannot be empty"));
    }
    if is_reserved_app_name(trimmed) {
        return Err(Error::new(format!(
            "'{trimmed}' is a switch command, so an app of that name could never be selected; \
             pick another name"
        )));
    }
    if trimmed.contains(['/', '\\']) {
        return Err(Error::new(format!(
            "application name '{trimmed}' cannot contain path separators"
        )));
    }
    Ok(())
}

/// Rejects profile names that cannot be used.
///
/// `switch <app> <word>` matches per-app subcommands before treating the word
/// as a profile, so a profile named `list` could never be switched to.
pub fn validate_profile_name(name: &str) -> Result<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::new("profile name cannot be empty"));
    }
    if is_reserved_profile_name(trimmed) {
        return Err(Error::new(format!(
            "'{trimmed}' is a switch subcommand, so a profile of that name could never be \
             selected; pick another name"
        )));
    }
    if trimmed.contains(['/', '\\']) {
        return Err(Error::new(format!(
            "profile name '{trimmed}' cannot contain path separators"
        )));
    }
    Ok(())
}

/// Finds an executable on a search path. Equivalent to `exec.LookPath`.
///
/// An explicitly empty `path_env` finds nothing, matching `PATH=""`.
fn look_path(name: &str, path_env: Option<&str>) -> Option<PathBuf> {
    let paths = match path_env {
        Some(p) => p.to_string(),
        None => std::env::var("PATH").unwrap_or_default(),
    };
    if paths.is_empty() {
        return None;
    }

    for dir in std::env::split_paths(&paths) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        for candidate in candidate_names(name) {
            let full = dir.join(&candidate);
            if is_executable(&full) {
                return Some(full);
            }
        }
    }
    None
}

#[cfg(windows)]
fn candidate_names(name: &str) -> Vec<String> {
    let mut names = vec![name.to_string()];
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    for ext in pathext.split(';').filter(|e| !e.is_empty()) {
        names.push(format!("{name}{ext}"));
    }
    names
}

#[cfg(not(windows))]
fn candidate_names(name: &str) -> Vec<String> {
    vec![name.to_string()]
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file())
        .unwrap_or(false)
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;
    use crate::config::AppConfig;

    /// Creates `~/.codex/auth.json` plus a backup per named profile.
    /// Port of the `setupCodexFiles` helper.
    pub fn setup_codex_files(home: &Path, auth_data: &str, accounts: &[(&str, &str)]) -> PathBuf {
        let codex_dir = home.join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let auth_path = codex_dir.join("auth.json");
        std::fs::write(&auth_path, auth_data).unwrap();
        for (name, data) in accounts {
            std::fs::write(switch_backup(&auth_path, name), data).unwrap();
        }
        auth_path
    }

    pub fn switch_backup(auth_path: &Path, name: &str) -> PathBuf {
        PathBuf::from(format!("{}.{}.switch", auth_path.display(), name))
    }

    /// A folder-backed app whose backups live outside the folder.
    pub fn folder_config(current: &str, accounts: &[&str]) -> AppConfig {
        AppConfig {
            current: current.to_string(),
            accounts: accounts.iter().map(|s| s.to_string()).collect(),
            auth_path: "~/App".to_string(),
            switch_pattern: "~/.switch/profiles/appx/{name}".to_string(),
        }
    }

    /// Creates `~/App` plus a backup folder per profile, each holding an
    /// `id.txt` naming it.
    pub fn setup_folder_app(home: &Path, live: &str, profiles: &[&str]) -> PathBuf {
        let app = home.join("App");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("id.txt"), live).unwrap();
        for name in profiles {
            let backup = home
                .join(".switch")
                .join("profiles")
                .join("appx")
                .join(name);
            std::fs::create_dir_all(&backup).unwrap();
            std::fs::write(backup.join("id.txt"), name).unwrap();
        }
        app
    }

    /// The codex app config the Go tests seed over and over.
    pub fn codex_config(current: &str, accounts: &[&str]) -> AppConfig {
        AppConfig {
            current: current.to_string(),
            accounts: accounts.iter().map(|s| s.to_string()).collect(),
            auth_path: "~/.codex/auth.json".to_string(),
            switch_pattern: "{auth_path}.{name}.switch".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::ctx::test_ctx;

    fn switcher(home: &Path) -> Switcher {
        Switcher::with_home(home).unwrap()
    }

    // Port of TestNewSwitcher_CreatesConfig.
    #[test]
    fn new_switcher_creates_config() {
        let home = tempfile::tempdir().unwrap();
        let s = switcher(home.path());

        assert_eq!(s.config_path, home.path().join(".switch.toml"));
        assert!(s.config_path.exists(), "config file not created");
        assert_eq!(s.config.default.config, "codex");
    }

    // Port of TestNewSwitcher_ConfigReadError.
    #[test]
    fn new_switcher_config_read_error() {
        let home = tempfile::tempdir().unwrap();
        // A directory where the config file belongs cannot be read.
        std::fs::create_dir_all(home.path().join(".switch.toml")).unwrap();
        assert!(Switcher::with_home(home.path()).is_err());
    }

    // Port of TestGetSetAppConfig.
    #[test]
    fn get_set_app_config() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());

        assert!(
            s.get_app_config("codex").is_none(),
            "expected no codex app yet"
        );

        let cfg = codex_config("", &[]);
        s.set_app_config("codex", cfg.clone());
        assert_eq!(s.get_app_config("codex").unwrap().auth_path, cfg.auth_path);
    }

    // Port of TestAddAccount_NewTemplateApp.
    #[test]
    fn add_account_new_template_app() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = setup_codex_files(home.path(), r#"{"token":"t123"}"#, &[]);
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());

        s.add_account(&mut ctx, "codex", "alice").unwrap();

        assert!(
            switch_backup(&auth_path, "alice").exists(),
            "switch backup missing"
        );
        let app = s.get_app_config("codex").unwrap();
        assert_eq!(app.current, "alice");
        assert!(app.accounts.contains(&"alice".to_string()));
    }

    // Port of TestAddAccount_Duplicate_NoOverwrite.
    #[test]
    fn add_account_duplicate_no_overwrite() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = setup_codex_files(
            home.path(),
            r#"{"token":"orig"}"#,
            &[("alice", r#"{"token":"old"}"#)],
        );
        let (mut ctx, out, _) = test_ctx(home.path(), "no\n");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("alice", &["alice"]));
        s.save().unwrap();

        let err = s.add_account(&mut ctx, "codex", "alice").unwrap_err();
        assert_eq!(err, Error::Cancelled);
        assert!(out.contents().contains("already exists"));

        // The existing backup must not have been overwritten.
        let backup = std::fs::read_to_string(switch_backup(&auth_path, "alice")).unwrap();
        assert!(
            backup.contains("old"),
            "switch file should remain old, got: {backup}"
        );
    }

    // Port of TestAddAccount_OverwriteYes.
    #[test]
    fn add_account_overwrite_yes() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = setup_codex_files(
            home.path(),
            r#"{"token":"NEW"}"#,
            &[("alice", r#"{"token":"OLD"}"#)],
        );
        let (mut ctx, _, _) = test_ctx(home.path(), "yes\n");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("alice", &["alice"]));
        s.save().unwrap();

        s.add_account(&mut ctx, "codex", "alice").unwrap();

        let live = std::fs::read_to_string(&auth_path).unwrap();
        let backup = std::fs::read_to_string(switch_backup(&auth_path, "alice")).unwrap();
        assert_eq!(live, backup, "overwrite did not copy current content");
    }

    // Port of TestAddAccount_SaveConfigError_RollsBack.
    #[test]
    fn add_account_save_config_error_rolls_back() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = setup_codex_files(home.path(), r#"{"token":"z"}"#, &[]);
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());

        // Pointing the config at a directory makes saving fail.
        let bad_dir = tempfile::tempdir().unwrap();
        s.config_path = bad_dir.path().to_path_buf();

        assert!(s.add_account(&mut ctx, "codex", "p1").is_err());
        assert!(
            !switch_backup(&auth_path, "p1").exists(),
            "expected the backup to be removed after a failed save"
        );
    }

    // Port of TestAddAccount_TemplateAuthMissing_Error.
    #[test]
    fn add_account_template_auth_missing_error() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());

        let err = s.add_account(&mut ctx, "codex", "x").unwrap_err();
        assert!(
            err.to_string().contains("auth path not found"),
            "expected auth path not found, got {err}"
        );
    }

    // Port of TestAddAccount_NoTemplateError.
    #[test]
    fn add_account_no_template_error() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());

        let err = s.add_account(&mut ctx, "unknownapp", "p").unwrap_err();
        assert!(
            err.to_string().contains("no configuration found"),
            "expected no configuration found error, got {err}"
        );
    }

    // Port of TestSwitchAccount_Success.
    #[test]
    fn switch_account_success() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#), ("b", r#"{"token":"b"}"#)],
        );
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        s.save().unwrap();

        s.switch_account(&mut ctx, "codex", "b").unwrap();

        let live = std::fs::read_to_string(&auth_path).unwrap();
        assert!(live.contains('b'), "auth not switched: {live}");
    }

    // Port of TestSwitchAccount_SameAccount.
    #[test]
    fn switch_account_same_account() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#)],
        );
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a"]));
        s.save().unwrap();

        s.switch_account(&mut ctx, "codex", "a").unwrap();

        let text = out.contents();
        assert!(
            text.contains("Switched to: a"),
            "expected 'Switched to: a', got {text:?}"
        );
    }

    // Port of TestSwitchAccount_NotFound.
    #[test]
    fn switch_account_not_found() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("", &["a"]));

        assert!(s.switch_account(&mut ctx, "codex", "missing").is_err());
    }

    // Port of TestSwitchAccount_SwitchFileNotFound.
    #[test]
    fn switch_account_switch_file_not_found() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"t":1}"#, &[]);
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("", &["ghost"]));
        s.save().unwrap();

        let err = s.switch_account(&mut ctx, "codex", "ghost").unwrap_err();
        assert!(
            err.to_string().contains("switch file not found"),
            "expected switch file not found, got {err}"
        );
    }

    // Port of TestSwitchAccount_EmptyDelegatesToCycle.
    #[test]
    fn switch_account_empty_delegates_to_cycle() {
        let home = tempfile::tempdir().unwrap();
        let auth_path =
            setup_codex_files(home.path(), r#"{"token":"a"}"#, &[("a", "{}"), ("b", "{}")]);
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        s.save().unwrap();

        s.switch_account(&mut ctx, "codex", "").unwrap();

        let live = std::fs::read_to_string(&auth_path).unwrap();
        assert!(live.contains("{}"), "expected switched content from backup");
    }

    // Port of TestCycleAccounts.
    #[test]
    fn cycle_accounts_walks_the_list_and_wraps() {
        let home = tempfile::tempdir().unwrap();
        let auth_path = setup_codex_files(
            home.path(),
            r#"{"token":"u1"}"#,
            &[
                ("u1", r#"{"token":"u1"}"#),
                ("u2", r#"{"token":"u2"}"#),
                ("u3", r#"{"token":"u3"}"#),
            ],
        );
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("u1", &["u1", "u2", "u3"]));
        s.save().unwrap();

        for expected in ["u2", "u3", "u1"] {
            s.cycle_accounts(&mut ctx, "codex").unwrap();
            let live = std::fs::read_to_string(&auth_path).unwrap();
            assert!(live.contains(expected), "expected {expected}, got {live}");
        }
    }

    // Port of TestCycleAccounts_NoAccounts_And_EmptyCurrent.
    #[test]
    fn cycle_accounts_no_accounts_and_empty_current() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"t":1}"#, &[("a", "{}"), ("b", "{}")]);
        let (mut ctx, _, err_out) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());

        s.set_app_config("codex", codex_config("", &[]));
        let err = s.cycle_accounts(&mut ctx, "codex").unwrap_err();
        assert_eq!(err.to_string(), "no accounts");
        assert!(err_out
            .contents()
            .contains("No accounts configured for codex"));

        // With no profile matching the live config, cycling starts at the first.
        s.set_app_config("codex", codex_config("", &["a", "b"]));
        s.cycle_accounts(&mut ctx, "codex").unwrap();
        assert_eq!(s.get_app_config("codex").unwrap().current, "a");
    }

    // Port of TestFindCurrentAccount.
    #[test]
    fn find_current_account() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(
            home.path(),
            r#"{"token":"u2data"}"#,
            &[
                ("u1", r#"{"token":"u1data"}"#),
                ("u2", r#"{"token":"u2data"}"#),
            ],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("", &["u1", "u2"]));

        assert_eq!(s.find_current_account("codex"), "u2");
    }

    // Port of TestFindCurrentAccount_None.
    #[test]
    fn find_current_account_none() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(
            home.path(),
            r#"{"token":"main"}"#,
            &[("a", "{}"), ("b", "{}")],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("", &["a", "b"]));

        assert_eq!(s.find_current_account("codex"), "");
    }

    // Port of TestFindCurrentAccount_MissingAuthPath.
    #[test]
    fn find_current_account_missing_auth_path() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("", &["a"]));

        assert_eq!(s.find_current_account("codex"), "");
    }

    #[test]
    fn find_current_account_unknown_app() {
        let home = tempfile::tempdir().unwrap();
        let s = switcher(home.path());
        assert_eq!(s.find_current_account("nosuch"), "");
    }

    // Port of TestListAccountsAndAllApps.
    #[test]
    fn list_accounts_and_all_apps() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(
            home.path(),
            r#"{"token":"u1"}"#,
            &[("u1", r#"{"token":"u1"}"#), ("u2", r#"{"token":"u2"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("u1", &["u1", "u2"]));
        s.config.default.config = "codex".to_string();
        s.save().unwrap();

        let (mut ctx, out, _) = test_ctx(home.path(), "");
        s.list_accounts(&mut ctx, "codex");
        let text = out.contents();
        assert!(text.contains("Codex"), "{text}");
        assert!(text.contains("u1"), "{text}");
        assert!(text.contains("u2"), "{text}");

        let (mut ctx, out, _) = test_ctx(home.path(), "");
        s.list_all_apps(&mut ctx);
        let text = out.contents();
        assert!(text.contains("Configured applications:"), "{text}");
        assert!(text.contains("codex"), "{text}");
    }

    // Port of TestListAllApps_Empty.
    #[test]
    fn list_all_apps_empty() {
        let home = tempfile::tempdir().unwrap();
        let s = switcher(home.path());
        let (mut ctx, out, _) = test_ctx(home.path(), "");

        s.list_all_apps(&mut ctx);
        assert!(out.contents().contains("No applications configured"));
    }

    // Port of TestListAccounts_Variants.
    #[test]
    fn list_accounts_variants() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());

        let (mut ctx, out, _) = test_ctx(home.path(), "");
        s.list_accounts(&mut ctx, "nosuch");
        assert!(out.contents().contains("No accounts configured for nosuch"));

        // An empty app name delegates to the full listing.
        s.set_app_config("codex", codex_config("", &[]));
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        s.list_accounts(&mut ctx, "");
        let text = out.contents();
        assert!(
            text.contains("Configured applications:")
                || text.contains("No applications configured"),
            "expected the app listing, got {text:?}"
        );
    }

    #[test]
    fn list_all_apps_is_sorted_by_name() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        for name in ["vscode", "codex", "ssh"] {
            s.set_app_config(name, codex_config("", &[]));
        }

        let (mut ctx, out, _) = test_ctx(home.path(), "");
        s.list_all_apps(&mut ctx);
        let text = out.contents();
        let codex = text.find("codex").unwrap();
        let ssh = text.find("ssh").unwrap();
        let vscode = text.find("vscode").unwrap();
        assert!(
            codex < ssh && ssh < vscode,
            "apps should list in name order: {text:?}"
        );
    }

    // Port of TestSetDefaultApp_Success.
    #[test]
    fn set_default_app_success() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("p1", &["p1"]));
        s.set_app_config(
            "vscode",
            AppConfig {
                current: "p1".to_string(),
                accounts: vec!["p1".to_string()],
                auth_path: "~/.vscode/User".to_string(),
                switch_pattern: "~/.vscode/profiles/{name}.switch".to_string(),
            },
        );
        s.config.default.config = "codex".to_string();
        s.save().unwrap();

        s.set_default_app(&mut ctx, "vscode").unwrap();
        assert_eq!(s.config.default.config, "vscode");

        // The change must survive a reload.
        let reloaded = switcher(home.path());
        assert_eq!(reloaded.config.default.config, "vscode");
    }

    // Port of TestSetDefaultApp_AppNotFound.
    #[test]
    fn set_default_app_app_not_found() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());

        let err = s.set_default_app(&mut ctx, "nonexistent").unwrap_err();
        assert!(
            err.to_string().contains("app 'nonexistent' not found"),
            "expected app not found error, got {err}"
        );
    }

    // Port of TestSetDefaultApp_FirstTime.
    #[test]
    fn set_default_app_first_time() {
        let home = tempfile::tempdir().unwrap();
        let (mut ctx, out, _) = test_ctx(home.path(), "");
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("p1", &["p1"]));
        s.config.default.config = String::new();
        s.save().unwrap();

        s.set_default_app(&mut ctx, "codex").unwrap();
        assert!(out.contents().contains("Default app set to codex"));
    }

    // ---- folder-backed profiles ----------------------------------------

    /// Folder profiles used to be indistinguishable, so cycling bounced between
    /// the first two entries and never reached the third.
    #[test]
    fn cycling_a_folder_app_visits_every_profile() {
        let home = tempfile::tempdir().unwrap();
        let live = setup_folder_app(home.path(), "a", &["a", "b", "c"]);
        let mut s = switcher(home.path());
        s.set_app_config("appx", folder_config("a", &["a", "b", "c"]));
        s.save().unwrap();

        let id = || std::fs::read_to_string(live.join("id.txt")).unwrap();
        for expected in ["b", "c", "a", "b"] {
            let (mut ctx, _, _) = test_ctx(home.path(), "");
            s.cycle_accounts(&mut ctx, "appx").unwrap();
            assert_eq!(id(), expected, "cycle should have reached {expected}");
        }
    }

    /// `list` used to name the first profile as current regardless of what was
    /// actually live.
    #[test]
    fn a_folder_app_reports_the_profile_that_is_really_live() {
        let home = tempfile::tempdir().unwrap();
        setup_folder_app(home.path(), "b", &["a", "b", "c"]);
        let mut s = switcher(home.path());
        s.set_app_config("appx", folder_config("", &["a", "b", "c"]));

        assert_eq!(s.find_current_account("appx"), "b");
    }

    /// Restoring a profile must not leave the outgoing profile's files behind.
    #[test]
    fn switching_a_folder_profile_does_not_leak_files() {
        let home = tempfile::tempdir().unwrap();
        let live = setup_folder_app(home.path(), "work", &["work", "personal"]);
        // Only the work profile has credentials.
        let work = home
            .path()
            .join(".switch")
            .join("profiles")
            .join("appx")
            .join("work");
        std::fs::write(work.join("credentials"), b"work-token").unwrap();
        std::fs::write(live.join("credentials"), b"work-token").unwrap();

        let mut s = switcher(home.path());
        s.set_app_config("appx", folder_config("work", &["personal", "work"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        s.switch_account(&mut ctx, "appx", "personal").unwrap();

        assert_eq!(
            std::fs::read_to_string(live.join("id.txt")).unwrap(),
            "personal"
        );
        assert!(
            !live.join("credentials").exists(),
            "the outgoing profile's credentials must not stay live"
        );
    }

    /// Re-capturing a profile replaces the backup instead of merging into it.
    #[test]
    fn re_adding_a_profile_replaces_the_backup() {
        let home = tempfile::tempdir().unwrap();
        let live = setup_folder_app(home.path(), "work", &["work"]);
        let backup = home
            .path()
            .join(".switch")
            .join("profiles")
            .join("appx")
            .join("work");
        std::fs::write(backup.join("stale.txt"), b"old").unwrap();

        let mut s = switcher(home.path());
        s.set_app_config("appx", folder_config("work", &["work"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "yes\n");
        s.add_account(&mut ctx, "appx", "work").unwrap();

        assert!(live.join("id.txt").exists());
        assert!(
            !backup.join("stale.txt").exists(),
            "a stale file should not survive a re-capture"
        );
    }

    // ---- unsafe layouts and migration ----------------------------------

    fn nested_config(accounts: &[&str]) -> AppConfig {
        AppConfig {
            current: String::new(),
            accounts: accounts.iter().map(|s| s.to_string()).collect(),
            auth_path: "~/.ssh".to_string(),
            // The shape the original ssh template used.
            switch_pattern: "~/.ssh/profiles/{name}.switch".to_string(),
        }
    }

    #[test]
    fn a_backup_inside_the_config_folder_is_refused() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
        std::fs::write(home.path().join(".ssh").join("id_rsa"), b"KEY").unwrap();

        let mut s = switcher(home.path());
        s.set_app_config("ssh", nested_config(&[]));

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let err = s.add_account(&mut ctx, "ssh", "work").unwrap_err();
        assert!(
            err.to_string().contains("is inside the config path"),
            "{err}"
        );
    }

    // ---- drift ----------------------------------------------------------

    #[test]
    fn a_config_edited_since_the_last_switch_is_reported_as_drifted() {
        let home = tempfile::tempdir().unwrap();
        let auth = setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#), ("b", r#"{"token":"b"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));

        assert_eq!(
            s.current_profile("codex"),
            Current::Matched("a".to_string())
        );

        // Edit the live config so it matches no backup.
        std::fs::write(&auth, br#"{"token":"edited"}"#).unwrap();
        assert_eq!(
            s.current_profile("codex"),
            Current::Drifted("a".to_string())
        );
        assert_eq!(s.find_current_account("codex"), "a");

        let (mut ctx, out, _) = test_ctx(home.path(), "");
        s.list_accounts(&mut ctx, "codex");
        assert!(
            out.contents().contains("(current, modified)"),
            "{}",
            out.contents()
        );
    }

    #[test]
    fn an_unrecorded_current_still_reports_nothing() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"token":"main"}"#, &[("a", "{}")]);
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("", &["a"]));

        assert_eq!(s.current_profile("codex"), Current::None);
    }

    // ---- error handling -------------------------------------------------

    /// A profile whose backup directory cannot be created, used to force the
    /// backup step to fail without relying on permissions.
    fn store_config(current: &str, accounts: &[&str]) -> AppConfig {
        AppConfig {
            current: current.to_string(),
            accounts: accounts.iter().map(|s| s.to_string()).collect(),
            auth_path: "~/.codex/auth.json".to_string(),
            switch_pattern: "~/store/{name}/data".to_string(),
        }
    }

    #[test]
    fn a_failed_backup_stops_the_switch() {
        let home = tempfile::tempdir().unwrap();
        let auth = setup_codex_files(home.path(), r#"{"token":"live"}"#, &[]);

        let other = home.path().join("store").join("other");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("data"), br#"{"token":"other"}"#).unwrap();
        // A file where the 'work' backup directory needs to be.
        std::fs::write(home.path().join("store").join("work"), b"blocked").unwrap();

        let mut s = switcher(home.path());
        s.set_app_config("codex", store_config("work", &["other", "work"]));
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "");
        let err = s.switch_account(&mut ctx, "codex", "other").unwrap_err();
        assert!(err.to_string().contains("could not back up"), "{err}");

        // The live config is untouched, which is the point of stopping.
        assert_eq!(
            std::fs::read_to_string(&auth).unwrap(),
            r#"{"token":"live"}"#
        );
    }

    #[test]
    fn a_failed_save_warns_but_the_switch_still_happened() {
        let home = tempfile::tempdir().unwrap();
        let auth = setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#), ("b", r#"{"token":"b"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        // A directory cannot be written as the config file.
        s.config_path = home.path().join("unwritable");
        std::fs::create_dir_all(&s.config_path).unwrap();

        let (mut ctx, _, err_out) = test_ctx(home.path(), "");
        s.switch_account(&mut ctx, "codex", "b").unwrap();

        assert!(
            std::fs::read_to_string(&auth).unwrap().contains('b'),
            "the switch should still happen"
        );
        assert!(
            err_out.contents().contains("could not be saved"),
            "{}",
            err_out.contents()
        );
    }

    // ---- removal --------------------------------------------------------

    #[test]
    fn remove_account_deletes_the_backup_after_confirmation() {
        let home = tempfile::tempdir().unwrap();
        let auth = setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#), ("b", r#"{"token":"b"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        s.save().unwrap();

        // Declining changes nothing.
        let (mut ctx, _, _) = test_ctx(home.path(), "no\n");
        assert_eq!(
            s.remove_account(&mut ctx, "codex", "b").unwrap_err(),
            Error::Cancelled
        );
        assert!(switch_backup(&auth, "b").exists());

        let (mut ctx, _, _) = test_ctx(home.path(), "yes\n");
        s.remove_account(&mut ctx, "codex", "b").unwrap();

        assert!(
            !switch_backup(&auth, "b").exists(),
            "the backup should be gone"
        );
        assert_eq!(
            s.get_app_config("codex").unwrap().accounts,
            vec!["a".to_string()]
        );
        // The live config is never touched by a removal.
        assert!(auth.exists());

        // And it survives a reload.
        assert_eq!(
            switcher(home.path())
                .get_app_config("codex")
                .unwrap()
                .accounts,
            vec!["a"]
        );
    }

    #[test]
    fn removing_the_profile_in_use_clears_the_recorded_current() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a"]));
        s.save().unwrap();

        let (mut ctx, out, _) = test_ctx(home.path(), "yes\n");
        s.remove_account(&mut ctx, "codex", "a").unwrap();

        assert!(
            out.contents().contains("currently in use"),
            "{}",
            out.contents()
        );
        assert_eq!(s.get_app_config("codex").unwrap().current, "");
    }

    #[test]
    fn remove_account_rejects_an_unknown_profile() {
        let home = tempfile::tempdir().unwrap();
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a"]));

        let (mut ctx, _, _) = test_ctx(home.path(), "yes\n");
        let err = s.remove_account(&mut ctx, "codex", "nope").unwrap_err();
        assert!(err.to_string().contains("not found"), "{err}");
    }

    #[test]
    fn remove_app_drops_every_backup_and_repoints_the_default() {
        let home = tempfile::tempdir().unwrap();
        let auth = setup_codex_files(
            home.path(),
            r#"{"token":"a"}"#,
            &[("a", r#"{"token":"a"}"#), ("b", r#"{"token":"b"}"#)],
        );
        let mut s = switcher(home.path());
        s.set_app_config("codex", codex_config("a", &["a", "b"]));
        s.set_app_config("other", codex_config("", &[]));
        s.config.default.config = "codex".to_string();
        s.save().unwrap();

        let (mut ctx, _, _) = test_ctx(home.path(), "yes\n");
        s.remove_app(&mut ctx, "codex").unwrap();

        assert!(s.get_app_config("codex").is_none());
        assert!(!switch_backup(&auth, "a").exists());
        assert!(!switch_backup(&auth, "b").exists());
        assert!(auth.exists(), "the live config is left alone");
        assert_eq!(
            s.config.default.config, "other",
            "the default should move on"
        );
    }

    // ---- name validation ------------------------------------------------

    #[test]
    fn reserved_and_malformed_app_names_are_rejected() {
        for name in [
            "list", "add", "help", "config", "default", "version", "rm", "RM", "-v",
        ] {
            assert!(
                validate_app_name(name).is_err(),
                "{name} should be rejected"
            );
        }
        assert!(validate_app_name("").is_err());
        assert!(validate_app_name("  ").is_err());
        assert!(validate_app_name("a/b").is_err());

        for name in ["codex", "my_app", "vscode2"] {
            assert!(validate_app_name(name).is_ok(), "{name} should be accepted");
        }
    }

    #[test]
    fn malformed_profile_names_are_rejected() {
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("   ").is_err());
        assert!(validate_profile_name("a/b").is_err());
        // `switch codex list` lists profiles, so a profile called "list"
        // could never be selected.
        for name in ["list", "add", "rm", "remove", "config", "-v"] {
            assert!(
                validate_profile_name(name).is_err(),
                "{name} should be rejected"
            );
        }
        assert!(validate_profile_name("work").is_ok());
        assert!(validate_profile_name("listing").is_ok());
    }

    #[test]
    fn add_account_rejects_an_empty_profile_name() {
        let home = tempfile::tempdir().unwrap();
        setup_codex_files(home.path(), r#"{"t":1}"#, &[]);
        let mut s = switcher(home.path());
        let (mut ctx, _, _) = test_ctx(home.path(), "");

        assert!(s.add_account(&mut ctx, "codex", "").is_err());
    }

    // Port of TestOpenConfig_WithEditor (skipped on Windows there).
    #[cfg(unix)]
    #[test]
    fn open_config_with_editor() {
        let home = tempfile::tempdir().unwrap();
        let s = switcher(home.path());
        // `echo` exists everywhere and exits 0.
        s.open_config_with(Some("echo"), None).unwrap();
    }

    // Port of TestOpenConfig_NoEditor.
    #[test]
    fn open_config_no_editor() {
        let home = tempfile::tempdir().unwrap();
        let s = switcher(home.path());

        let err = s.open_config_with(Some(""), Some("")).unwrap_err();
        assert!(
            err.to_string().contains("no text editor found"),
            "expected no text editor error, got {err}"
        );
    }

    // Port of TestOpenConfig_EditorNotFound.
    #[test]
    fn open_config_editor_not_found() {
        let home = tempfile::tempdir().unwrap();
        let s = switcher(home.path());
        assert!(s
            .open_config_with(Some("nonexistent-editor-12345"), Some(""))
            .is_err());
    }

    // Port of TestOpenConfig_EditorDetection.
    #[test]
    fn open_config_editor_detection() {
        let home = tempfile::tempdir().unwrap();
        let s = switcher(home.path());

        let bin_dir = home.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();

        let (editor_name, content) = if cfg!(windows) {
            ("nano.bat", "@echo off\necho fake editor\n")
        } else {
            ("nano", "#!/bin/sh\necho 'fake editor'\n")
        };
        let editor_path = bin_dir.join(editor_name);
        std::fs::write(&editor_path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&editor_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        // With no EDITOR set, the search finds the fake nano on the given path.
        s.open_config_with(Some(""), Some(bin_dir.to_str().unwrap()))
            .unwrap();
    }
}
