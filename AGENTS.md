# Agent instructions

These apply to every automated agent and every LLM-assisted change in this
repository. `CLAUDE.md` is a symlink to this file; edit this one.

## No LLM attribution, ever

Commits, pull requests, and code in this repository are authored by the person
driving the tool. Never attribute work to, or record co-authorship for, an LLM
or AI assistant — in any form:

- No `Co-Authored-By:` trailer naming a model, assistant, or AI vendor.
- No `Claude-Session:`, `Generated with`, `🤖`, or similar markers in commit
  messages, pull request descriptions, or code comments.
- No model or vendor names anywhere in a commit message or PR body.

Commit with the git identity of the person running the tool. If a harness,
plugin, or template tries to append its own attribution, strip it before
committing. If history has picked some up, rewrite it out.

## Project

`switch` is a Rust CLI that swaps configuration profiles for other tools by
copying a live config file or folder to and from named backups.

### Commands

```
cargo test                                  # unit + integration tests
cargo clippy --all-targets -- -D warnings   # must be clean
cargo fmt --all --check                     # must be clean
make dev                                    # fmt + clippy + test + release build
```

CI runs all three on Linux, macOS and Windows. Do not push with any of them
failing.

### Layout

- `src/cli.rs` — argument dispatch; returns exit codes rather than calling
  `exit`, so every path is testable in-process.
- `src/switcher.rs` — add / switch / cycle / remove / list / default.
- `src/wizard.rs` — the interactive `switch add` flow.
- `src/fsops.rs` — copying, and `replace_path` for atomic replace-not-merge.
- `src/compare.rs` — content comparison; JSON is order-insensitive, folders
  compare by tree.
- `src/paths.rs` — `~` expansion, lexical cleaning, pattern resolution.
- `src/config.rs` — `~/.switch.toml`; the `[default]` / `[apps.<name>]` layout
  is the on-disk format and must not change without a migration.
- `src/ctx.rs` — injected stdin/stdout/stderr, home directory, colours.
- `tests/cli.rs` — spawns the real binary via `CARGO_BIN_EXE_switch`.

### Conventions

- All I/O and the home directory go through `Ctx`. Never read `HOME`,
  `EDITOR`, or `PATH` directly in library code, and never mutate process
  environment variables in tests — spawn the binary with `Command::env`
  instead.
- Profile data is replaced, never merged: use `replace_path`, not `copy_path`,
  for anything the user will restore.
- A backup must never live inside the folder it backs up. `check_layout`
  refuses it; keep it that way.
- Diagnostics that accompany a non-zero exit go to stderr; output of a
  successful command goes to stdout.
- Colour is decided once in `Ctx::real()` from `NO_COLOR`, `CLICOLOR_FORCE`,
  and whether stdout is a terminal. Use `ctx.colors`, never raw escapes.
- App and profile names cannot collide with subcommands; the reserved lists
  live in `cli.rs` next to the dispatch they protect.
