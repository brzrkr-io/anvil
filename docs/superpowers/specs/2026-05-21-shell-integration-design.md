# Shell Integration — Design Spec

> Status: draft for review. Sub-project 4 of 4 (final) in milestone **M2**.
> Created: 2026-05-21. Owner-approved direction via brainstorm answers.

## Goal

Ship the shell-side scripts that make a spawned shell emit OSC 133 semantic
prompt marks and OSC 7 working-directory reports — and load them into the
shell automatically for zsh, opt-in for bash.

## Context

M1 already made `Terminal` *parse* OSC 133 (`promptMarks()`) and OSC 7
(`cwd()`). Nothing emits them today because the spawned login shell runs the
user's plain config. This sub-project closes that loop. `Pty.buildChildEnv`
builds the child environment from the app's own `environ`, so the app can
inject integration env vars simply by `setenv`-ing them before spawning shells
— no `Pty` API change.

## Decisions (settled with the owner)

1. **Delivery** — the two shell scripts are `@embedFile`d into the binary and
   written to a runtime dir at startup; the binary stays self-contained.
2. **zsh** — auto-injected (zero user setup) via a `ZDOTDIR` shim.
3. **bash** — opt-in: the app exports markers; the user adds one documented
   line to `~/.bashrc`.
4. **Config** — `config.zon` gains `shell_integration: bool = true`; `false`
   disables the `ZDOTDIR` injection.
5. **Scope** — emit the sequences. *Using* the marks (jump-to-prompt,
   command-region UI) is later (M3+).

## What the integration emits

OSC 133 semantic prompt marks, via the shell's prompt hooks:

| Mark | Meaning | When |
|---|---|---|
| `OSC 133 ; A ST` | prompt start | before each prompt is drawn |
| `OSC 133 ; B ST` | prompt end / input start | end of the prompt string |
| `OSC 133 ; C ST` | command start | just before a command runs |
| `OSC 133 ; D ; <exit> ST` | command done | after a command, with its exit code |

(`ST` = `\a` or `ESC \`. The scripts use the `\a` form, which `Terminal`'s
parser accepts.)

OSC 7 working directory: `OSC 7 ; file://<host><abs-path> ST`, emitted at each
prompt so `cd` is reflected.

## Components

```
shell/anvil-integration.zsh    zsh hook script (repo source of truth)
shell/anvil-integration.bash   bash hook script (repo source of truth)
src/app/shell_integration.zig    embeds both, writes them + the zsh ZDOTDIR
                                 shim at startup, assembles the env vars
src/main.zig                     calls shell_integration setup at startup
                                 (before the first tab spawns)
src/config/config.zig            + shell_integration: bool field
```

### The zsh scripts

**`anvil-integration.zsh`** — registers the hooks:
- Appends a function to `precmd_functions` that prints `OSC 133 ; D ; $?`
  (the just-finished command's exit), then `OSC 133 ; A`, then the OSC 7 cwd.
- Appends a function to `preexec_functions` that prints `OSC 133 ; C`.
- Ensures `OSC 133 ; B` is emitted at the end of the prompt — by idempotently
  appending the `B` escape to `PS1` (guarded so it is appended once, surviving
  the user's own `PS1` assignment because the guard/append runs from `precmd`).
- All escape output is wrapped so it does not corrupt prompt width
  (`%{...%}` zero-width markers where it sits in `PS1`).

**The `ZDOTDIR` shim** — a single `.zshenv` file written into the Caldera
runtime dir. zsh reads `$ZDOTDIR/.zshenv` first; the shim:
1. Restores `ZDOTDIR` to `${ANVIL_REAL_ZDOTDIR:-$HOME}` and unsets the marker,
   so the rest of zsh startup (`.zprofile`/`.zshrc`/`.zlogin`) reads the user's
   real files normally.
2. Sources the user's real `.zshenv` if present.
3. Sources `anvil-integration.zsh`.

Because the hooks are registered in `.zshenv`, they are in place before the
first prompt regardless of what the user's `.zshrc` does. Only one shim file is
needed (`.zshenv`); the user's other startup files run untouched.

### The bash script

**`anvil-integration.bash`** — the same marks via bash mechanisms:
`PROMPT_COMMAND` for the `D`/`A`/OSC-7 emission, a `DEBUG` trap for `C`, and a
`PS1` suffix for `B`. It is opt-in: the app exports the markers below; the user
adds one line to `~/.bashrc`:

```bash
[ -n "$ANVIL" ] && [ -r "$ANVIL_SHELL_INTEGRATION" ] && . "$ANVIL_SHELL_INTEGRATION"
```

### `shell_integration.zig`

```zig
/// Set up shell integration: write the embedded scripts + the zsh ZDOTDIR
/// shim into the runtime dir, and export the env vars that wire spawned
/// shells to them. Call once at startup, before any tab spawns. A no-op
/// (returns without exporting ZDOTDIR) when `enabled` is false.
pub fn setup(enabled: bool) void;
```

`setup`:
1. Resolves the runtime dir `~/.cache/anvil/shell/` (from `$HOME`);
   creates it.
2. Writes three files from `@embedFile` content: `anvil-integration.zsh`,
   `anvil-integration.bash`, and the `.zshenv` shim.
3. Exports, via `setenv`:
   - `ANVIL=1` — always (also a marker programs can detect).
   - `ANVIL_SHELL_INTEGRATION=<dir>/anvil-integration.bash` — for the bash
     opt-in line.
   - When `enabled` and zsh injection applies: `ANVIL_REAL_ZDOTDIR=<current
     ZDOTDIR or $HOME>`, then `ZDOTDIR=<runtime dir>`.
4. On any failure (no `$HOME`, dir create fails, write fails): log one line to
   stderr and return without exporting `ZDOTDIR` — the app still runs, just
   without integration. Never fatal.

`Pty.buildChildEnv` already copies `environ` into each child, so every shell
spawned after `setup` inherits these vars.

## Data flow

Startup (`main`): load config → `shell_integration.setup(cfg.shell_integration)`
→ then build the `TabManager` / first tab. Every tab's shell inherits the env.

A zsh tab: zsh reads the Caldera `.zshenv` shim → restores `ZDOTDIR`, sources
the user's config and the integration → hooks registered → every prompt emits
`D`/`A`/OSC-7, every command emits `C`, the prompt carries `B`. `Terminal`
records the marks (`promptMarks()`) and the cwd (`cwd()`).

## Error handling

| Situation | Behavior |
|---|---|
| `$HOME` unset / cache dir uncreatable / file write fails | Log to stderr; skip `ZDOTDIR` export; app runs without integration. |
| `shell_integration = false` in config | `setup` still writes the scripts and exports `ANVIL` / `ANVIL_SHELL_INTEGRATION` (bash opt-in still works); only `ZDOTDIR` injection is skipped. |
| User already has `ZDOTDIR` set | Captured into `ANVIL_REAL_ZDOTDIR`; the shim restores it. |
| Shell is not zsh (bash/other) | `ZDOTDIR` is harmless to a non-zsh shell; bash uses the opt-in line; other shells are unaffected. |
| Integration script has an error | Shell startup continues — the scripts guard their own commands; a broken hook must not break the shell. |

## Testing

`zig build test` is the gate; sub-project 1-3 tests stay green. Unit tests
(`shell_integration.zig`):
- The runtime-dir path is resolved correctly from `$HOME`.
- `setup(false)` exports no `ZDOTDIR`.
- `setup(true)` exports `ZDOTDIR`, `ANVIL_REAL_ZDOTDIR`, `ANVIL`,
  `ANVIL_SHELL_INTEGRATION`, and the three files exist with non-empty content
  afterward (use a temp `$HOME` override for the test).
- A pre-existing `ZDOTDIR` is preserved into `ANVIL_REAL_ZDOTDIR`.

The scripts themselves are integration-verified (`zig build run`): after a
prompt and a command, `Terminal.promptMarks()` is non-empty and `cwd()` is set;
opening a new tab inherits the current directory.

## Out of scope (deliberate)

- fish and other non-zsh/bash shells.
- *Consuming* the marks — jump-to-prompt, command-region selection, an
  AI-readable command/output API. That is M3+.
- A bundled `.app` and a GUI installer for the bash line.
- Re-emitting marks for already-running shells.

## File summary

| File | Change |
|---|---|
| `shell/anvil-integration.zsh` | Create — zsh hooks. |
| `shell/anvil-integration.bash` | Create — bash hooks. |
| `src/app/shell_integration.zig` | Create — embed, write, env-var setup. |
| `src/config/config.zig` | Modify — add `shell_integration: bool = true`. |
| `src/main.zig` | Modify — call `shell_integration.setup` at startup. |
| `src/pty/pty.zig` | Unchanged — `buildChildEnv` already propagates `environ`. |
