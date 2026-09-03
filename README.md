# switch

Switch between configuration profiles for the tools you use — one command to
swap accounts in Codex, Claude Code, VS Code, Cursor, SSH, Git, or anything
else that keeps its settings in a file or folder.

`switch` saves the live config as a named profile and restores it on demand.
Profiles are plain copies on disk; nothing is encrypted, synced, or sent
anywhere.

This is a Rust rewrite. The idea and the original command set came from
[surajmandalcell/switch](https://github.com/surajmandalcell/switch) by Suraj
Mandal, which was the inspiration for this project.

## Quick start

```bash
make install                  # builds and copies to /usr/local/bin

# log in to account A, then capture it
switch add codex work

# log in to account B, then capture that too
switch add codex personal

# from now on, one command flips between them
switch                        # cycle the default app
switch codex work             # or name the profile
```

## Install

### Prebuilt binaries

Every tagged release (`v*`), and any manual run of the
[release-binaries workflow](https://github.com/noeljackson/switch/actions/workflows/release-binaries.yml),
uploads one archive per platform as a run artifact:

| Platform | Archive |
|---|---|
| Linux x86_64 (static, musl) | `switch-<version>-linux-amd64.tar.gz` |
| macOS Apple Silicon | `switch-<version>-darwin-arm64.tar.gz` |
| Windows x86_64 | `switch-<version>-windows-amd64.zip` |

Unpack and put `switch` (or `switch.exe`) somewhere on your `PATH`.

### From source

Requires Rust 1.74 or newer.

```bash
git clone https://github.com/noeljackson/switch
cd switch

cargo install --path .           # into ~/.cargo/bin
# or
make install                     # /usr/local/bin, uses sudo
make install-user                # ~/bin
```

Other targets build with `cargo build --release --target <triple>` once the
target is added with `rustup target add`. macOS and Windows binaries have to be
built on those platforms (or with a cross toolchain); the release workflow uses
one runner per platform for that reason.

## Usage

```
switch                         Cycle through the default app's profiles
switch <app>                   Cycle through an app's profiles
switch <app> <profile>         Switch to a specific profile
switch add                     Interactive setup wizard
switch add <app>               Add a profile (prompts for the name)
switch add <app> <profile>     Save the current config as a profile
switch rm <app> <profile>      Remove a profile and its backup
switch rm <app>                Remove an app and all of its backups
switch list                    List apps and profiles
switch list <app>              List one app's profiles
switch default <app>           Set the default app
switch config                  Open ~/.switch.toml in $EDITOR
switch -h, --help              Usage
switch -v, --version           Version
```

`switch <app> add|rm|list|config …` work too, for muscle memory that starts
with the app name.

Removals ask for confirmation and only ever delete stored backups; the live
config is never touched. Errors and usage hints go to stderr, so
`switch list > file` is clean.

### Which profile am I on?

`switch` compares the live config against each stored profile instead of
trusting a recorded name, so a config you edited by hand — or that the app
rewrote — is still identified correctly. JSON compares regardless of key order;
folders compare by their whole tree. If nothing matches, listings show the
profile you last switched to, marked `(current, modified)`.

Cycling starts from the detected profile, so it always advances to the next
one.

## Built-in templates

`switch add` detects these automatically. Each captures either a single file or
an entire folder.

| App | What is captured | Backups |
|---|---|---|
| `codex` | `~/.codex/auth.json` | beside the file |
| `claude` | `~/.claude/config.json` | beside the file |
| `claudecode` | `~/.claude/settings.json` | beside the file |
| `git` | `~/.gitconfig` | beside the file |
| `vscode` | `~/.vscode/User` (folder) | `~/.switch/profiles/vscode/` |
| `cursor` | `~/.cursor` (folder) | `~/.switch/profiles/cursor/` |
| `ssh` | `~/.ssh` (folder) | `~/.switch/profiles/ssh/` |
| `antigravity` | `~/Library/Application Support/Antigravity` (folder) | `~/.switch/profiles/antigravity/` |

Where an app has more than one conventional location (VS Code and Cursor on
macOS, Antigravity on Linux) the wizard uses whichever it finds. Anything not
listed can be set up through the wizard's manual option: give it a path and a
profile name and it works the same way.

## Configuration

Everything lives in `~/.switch.toml`:

```toml
[default]
config = "codex"

[apps.codex]
current = "work"
accounts = ["personal", "work"]
auth_path = "~/.codex/auth.json"
switch_pattern = "{auth_path}.{name}.switch"

[apps.ssh]
current = "home"
accounts = ["home", "work"]
auth_path = "~/.ssh"
switch_pattern = "~/.switch/profiles/ssh/{name}"
```

`switch_pattern` decides where a profile's backup goes. It accepts two
placeholders: `{auth_path}`, the expanded config path, and `{name}`, the
profile name. A leading `~` in any path expands to your home directory.

A backup can never live inside the folder it backs up — `switch` refuses such a
pattern rather than copy a directory into itself.

App and profile names cannot be the same as a subcommand (`list`, `add`, `rm`,
and so on), since they could never be selected; both are checked when created.

## Where things are stored

- `~/.switch.toml` — which apps and profiles exist, and the default app.
- `<config path>.<profile>.switch` — backups of file-based configs, beside the
  original.
- `~/.switch/profiles/<app>/<profile>` — backups of folder-based configs.
  `~/.switch` is created with owner-only permissions, since profiles can hold
  credentials.

Backups are exact copies, permissions included. Restoring a profile replaces the
live config outright rather than merging into it, so a profile that lacks some
file does not inherit it from the previous one. Writes are staged and renamed
into place, so an interrupted run leaves either the old content or the new,
never a mix.

## Colour and environment

- Colour is on when stdout is a terminal and off when piped. `NO_COLOR` turns
  it off; `CLICOLOR_FORCE=1` turns it on regardless.
- `switch config` opens the config in `$EDITOR`, falling back to `nano`, `vi`,
  `vim`, `code`, or `gedit`, whichever is found first.
- `HOME` (`USERPROFILE` on Windows) decides where `~` points.

## Development

```bash
cargo test                                  # unit + integration tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
make dev                                    # all of the above, then a release build
```

CI runs the same checks on Linux, macOS, and Windows. The module layout and the
invariants the code depends on are in [AGENTS.md](AGENTS.md).

`switch -v` prints the version stamped at build time: the git commit or tag,
or `SWITCH_VERSION` if that was set when building.

## License

MIT — see [LICENSE](LICENSE).
