---
status: active
type: concept
created: 2026-05-21
updated: 2026-05-29
sources:
  - ../../src/app.zig
  - ../../src/session.zig
  - ../../src/vt/terminal.zig
  - ../../src/config.zig
confidence: high
---

# Shell Integration

Anvil emits OSC 133 semantic prompt marks and OSC 7 working-directory
reports from the running shell. These sequences let the terminal model know
where each prompt starts and ends, when a command is running, what the exit
status is, and what the current working directory is. New tabs inherit the
active tab's cwd.

## OSC 133 Prompt Marks

OSC 133 (the "FinalTerm" / iTerm2 semantic prompt protocol) uses four mark
variants, all terminated with `BEL` (`\a`):

| Sequence | Meaning |
|----------|---------|
| `\e]133;A\a` | Prompt start — the shell is about to draw the prompt |
| `\e]133;B\a` | Prompt end / input start — the user is typing a command |
| `\e]133;C\a` | Command start — a command is about to execute |
| `\e]133;D;<exit>\a` | Command end — the command finished with `<exit>` status |

The terminal parser (`src/vt/terminal.zig`) already handled these marks
from M1; shell integration makes the shell actually emit them.

## OSC 7 Working Directory

`\e]7;file://<host><path>\a` reports the shell's current working directory.
`Terminal` stores the raw value in `cwd_buf` (1024 bytes). `cwd()` returns the
raw URL; `cwd_path()` strips the `file://` prefix and the host component
(everything up to and including the first `/` after `file://`) and returns a
plain filesystem path as a sub-slice of `cwd_buf` with no allocation. Bare
paths and empty values pass through unchanged.

`app.zig` reads `cwd_path()` on the active session's terminal; `add_tab` passes
the result to `SessionManager.addTab` so the new shell starts in the same directory.

## Zsh Integration Script

`shell/anvil-integration.zsh` uses zsh's built-in hook arrays:

- `precmd_functions` — runs before every prompt draw. `__anvil_precmd`
  emits `133;D;<exit>` (command end), then `OSC 7` (cwd report), then
  `133;A` (prompt start).
- `preexec_functions` — runs just before each command executes.
  `__anvil_preexec` emits `133;C` (command start).
- A one-shot `__anvil_mark_prompt` precmd appends the `133;B` sequence to
  `PS1` as a zero-width `%{...%}` segment after the user's `.zshrc` has
  finished setting `PS1`. It then removes itself from `precmd_functions`.
  The `133;B` guard uses `$'...'` as a standalone token (not inside a
  double-quoted string) so zsh interprets the escape correctly.
- A guard variable (`ANVIL_ZSH_LOADED`) prevents double-sourcing.

## Zsh ZDOTDIR Shim (Auto-injection)

When `shell_integration` is enabled, `setup` sets `ZDOTDIR` to the Anvil
runtime directory (`~/.cache/anvil/shell`) before any shell spawns.
zsh reads `$ZDOTDIR/.zshenv` first on startup, so the shim
(`shell/zdotdir-zshenv.zsh`) runs before the user's own startup files.

The shim:
1. Restores `ZDOTDIR` to `$ANVIL_REAL_ZDOTDIR` (if set) or `$HOME`, so the
   rest of zsh startup (`.zprofile`, `.zshrc`, `.zlogin`) reads the user's
   own files.
2. Sources the user's own `~/.zshenv` if it exists.
3. Sources `anvil-integration.zsh` via `$ANVIL_SHELL_INTEGRATION_ZSH`.

If `ZDOTDIR` was already set before Anvil ran, `setup` stashes it in
`ANVIL_REAL_ZDOTDIR` so the shim can restore it.

## Bash Integration Script

`shell/anvil-integration.bash` is opt-in. The user adds one line to
`~/.bashrc`:

```bash
[ -n "$ANVIL" ] && [ -r "$ANVIL_SHELL_INTEGRATION" ] && . "$ANVIL_SHELL_INTEGRATION"
```

The script uses `PROMPT_COMMAND` (wraps it, prepending `__anvil_prompt_wrapper`)
and the `DEBUG` trap to approximate zsh's `precmd`/`preexec` behaviour.
A guard `$__anvil_in_prompt` suppresses the `DEBUG` trap while
`PROMPT_COMMAND` itself runs, so only real commands emit `133;C`.
`133;B` is appended to `PS1` once via a `case` statement.

## `shell_integration` Config Toggle

The `Config` struct (`src/config.zig`) has a `shell_integration: bool`
field (default `true`). When set to `false` in `config.toml`:

```toml
shell_integration = false
```

`setup(false)` still writes the three script files and exports the
`ANVIL` and `ANVIL_SHELL_INTEGRATION` marker variables, but skips
the `ZDOTDIR` override. zsh auto-injection is disabled; the bash opt-in line
still works if the user has it in `~/.bashrc`.

## Embed-and-Write at Startup

Shell scripts are embedded via `@embedFile` in `src/app.zig` and written to
`~/.cache/anvil/shell` at startup. Any filesystem failure is logged and
degrades gracefully — integration is skipped but the app continues normally.

`app.zig` calls the shell integration setup after the config is loaded and
before any child shell is spawned, so the env vars are in place before the
shell inherits its environment via `src/pty.zig`.

## New-Tab CWD Inheritance

When the user opens a new tab (default `⌘T`), `app.zig` reads `Terminal.cwd_path()`
from the active session and passes the result to `SessionManager.addTab`, which
`chdir`s the terminal process into that directory before `exec`ing the shell.
The shell therefore starts in the same directory as the tab the user was just working in.

## Modules

- `src/app.zig` — `current_cwd()`, `add_tab`, startup shell integration setup
- `src/session.zig` — session lifecycle; PTY env setup
- `src/pty.zig` — forkpty + child env; `chdir` before exec
- `src/vt/terminal.zig` — `cwd()` / `cwd_path()` accessors; OSC 7 and OSC 133 parsing
- `src/config.zig` — `Config.shell_integration: bool`
