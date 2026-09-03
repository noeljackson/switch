# Switch

Simple CLI to switch between multiple profiles for any app that uses a file or folder for its configuration (e.g., Codex, Claude, VSCode, Cursor, SSH, Git).

## Overview

Store multiple profiles and swap your active config with one command. Works for both single files (like `~/.codex/auth.json`) and entire folders (like `~/.vscode/User`).

## Download prebuilt binaries

Continuous integration builds upload platform-specific archives for each successful run. You can download the latest artifacts here:

- Latest CI build artifacts: [release-binaries workflow](https://github.com/surajmandalcell/switch/actions/workflows/release-binaries.yml)

Artifact URLs are tied to individual runs, so open the most recent successful run to grab the archive for your platform. If you prefer to build locally, follow the setup instructions below.

## TL;DR for quick usage

```
git clone https://github.com/surajmandalcell/switch && cd switch
make install

# login to one of your codex cli account and then
switch add codex codex1

# logut of current codex cli account and login to new one and then
switch add codex codex2

# now you can freely switch between them by just one, if you have more it behaves the same
switch
# or
switch codex codex1
```

## Features

- **App‑agnostic**: Works with any file/folder config
- **Built‑in templates**: Codex, Claude, VSCode, Cursor, SSH, Git
- **Wizard setup**: `switch add` guides detection and setup
- **Cycle or target**: Cycle profiles or switch to a specific one
- **Folder support**: Back up and restore whole config directories

## Installation

### From Source

```bash
git clone https://github.com/surajmandalcell/switch.git
cd switch
cargo build --release
```

The binary lands at `target/release/switch`. `make build` does the same and
copies it to `./build/switch`.

### Install Globally

```bash
# System-wide installation
sudo cp ./build/switch /usr/local/bin/

# Or install to user's ~/bin directory
mkdir -p ~/bin
cp ./build/switch ~/bin/
echo 'export PATH="$HOME/bin:$PATH"' >> ~/.zshrc  # or ~/.bashrc
source ~/.zshrc
```

### Cross-Platform Builds

Add the target once with `rustup target add <target>`, then build:

```bash
# macOS
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# Linux (musl produces a fully static binary)
cargo build --release --target x86_64-unknown-linux-musl
cargo build --release --target aarch64-unknown-linux-musl

# Windows
cargo build --release --target x86_64-pc-windows-msvc
```

Rust links against the host platform's toolchain, so macOS and Windows targets
need to be built on those platforms (or with a cross-compilation toolchain such
as `cross`). The release workflow uses one runner per platform for that reason.

## Quick Start

### 1. Add your first app/profile

```bash
switch add
# Wizard will auto-detect known apps or let you set up manually
```

### 2. Switch between profiles

```bash
# Cycle default app
switch

# Cycle specific app
switch codex

# Switch to a specific profile
switch codex work

# List apps and profiles
switch list
switch list codex
```

## Usage

### Commands

- `switch`: Cycle the default app
- `switch <app>`: Cycle profiles for an app
- `switch <app> <profile>`: Switch to a profile
- `switch add`: Launch setup wizard
- `switch add <app>`: Add a profile to an app (prompts for name)
- `switch add <app> <profile>`: Add current config as a profile
- `switch rm <app> <profile>` (or `switch <app> rm <profile>`): Remove a profile and its backup
- `switch rm <app>`: Remove an app and all its backups
- `switch list` / `switch list <app>`: List apps or profiles
- `switch default <app>`: Set default app
- `switch config`: Open config file in editor
- `switch <app> config`: Open config file in editor
- `switch -h` / `--help`: Show usage
- `switch -v` / `--version`: Print the short version

Removals always ask for confirmation, and never touch the live config — only
the stored profile backups.

App and profile names cannot shadow a subcommand: an app called `list` or a
profile called `config` could never be selected, so both are rejected up front.

Errors and usage hints go to stderr; everything else goes to stdout.

### Examples

```bash
# Quick account switching
switch                    # Cycles to next account

# Specific account switching
switch codex work         # Switches to 'work' account
switch codex personal     # Switches to 'personal' account

# Account management
switch add codex staging  # Saves current auth.json as 'staging'
switch list codex         # Shows all accounts with current indicator
switch rm codex staging   # Deletes the 'staging' backup (asks first)

# Configuration management
switch default codex      # Sets codex as the default app
switch config             # Opens ~/.switch.toml in your editor
switch codex config       # Alternative way to open config
```

## Where things are stored

- `~/.switch.toml` — which apps and profiles exist, and which is the default.
- `~/.switch/profiles/<app>/<profile>` — backups for folder-based apps
  (VSCode, Cursor, SSH, Antigravity). Created with owner-only permissions,
  since profiles can contain credentials.
- `<config path>.<profile>.switch` — backups for file-based apps, kept beside
  the file they came from (`~/.codex/auth.json.work.switch`).

Backups for a folder are never written inside that folder: a backup nested in
its own source cannot be copied or restored correctly, so `switch` refuses a
`switch_pattern` that would do it.

## Which profile am I on?

`switch` works this out by comparing the live config against each stored
profile, so a config edited by hand is still identified correctly. JSON files
compare regardless of key order. If the live config matches no profile, listings
show the last profile you switched to marked `(modified)`.

## Colours

Colour is used when output is a terminal. It is disabled when output is piped or
redirected, when `NO_COLOR` is set, and enabled anyway by `CLICOLOR_FORCE=1`.

## Configuration

Config is stored at `~/.switch.toml`.

Example:

```toml
[default]
config = "codex"

[apps.codex]
current = "work"
accounts = ["work", "personal"]
auth_path = "~/.codex/auth.json"
switch_pattern = "{auth_path}.{name}.switch"

[apps.vscode]
current = "dev"
accounts = ["dev", "personal"]
auth_path = "~/.vscode/User"
switch_pattern = "~/.switch/profiles/vscode/{name}"
```

`switch_pattern` accepts two placeholders: `{auth_path}`, the expanded config
path, and `{name}`, the profile name.

## Development

### Testing

```bash
make test
```

### Building

```bash
cargo build --release
```

`make dev` runs formatting, lints, tests and a release build in one go.

### Contributing

PRs welcome.

## Requirements

- Rust 1.74 or newer (stable)
- Write access to your home directory

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Author

**Suraj Mandal**

- GitHub: [@surajmandalcell](https://github.com/surajmandalcell)
- Project: [github.com/surajmandalcell/switch](https://github.com/surajmandalcell/switch)
