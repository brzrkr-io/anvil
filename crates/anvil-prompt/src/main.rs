//! anvil-prompt — renders the Anvil shell prompt. Invoked by the shell on
//! every prompt draw.
//!
//! Args:
//!   --exit <n>          Exit code of the previous command (default 0).
//!   --transient         Emit the collapsed transient prompt instead.
//!   --shell <zsh|bash|plain>
//!   --width <n>         Terminal column width; used for right-aligned segment.
//!   --duration-ms <n>   Previous command duration in milliseconds (optional).
//!   --agent             Emit the agent-mode pipe rail prompt.
//!   --ctx-percent <n>   Agent context meter percentage (default 73).
//!   --token-percent <n> Agent token meter percentage (default 48).
//!
//! Env:
//!   ANVIL_AGENT_MODE=1 enables the agent prompt without passing --agent.
//!   ANVIL_AGENT_CTX_PERCENT / ANVIL_AGENT_TOKEN_PERCENT set meter values.
//!
use anvil_prompt_core::build_segments::{Inputs, assemble};
use anvil_prompt_core::context::detect;
use anvil_prompt_core::git::query;
use anvil_prompt_core::render::{self, AgentOptions, Options, Shell};

struct Args {
    exit_code: u8,
    transient: bool,
    agent: bool,
    shell: Shell,
    width: u16,
    duration_ms: Option<u64>,
    context_percent: u8,
    token_percent: u8,
}

fn parse_args() -> Args {
    let mut a = Args {
        exit_code: 0,
        transient: false,
        agent: std::env::var("ANVIL_AGENT_MODE").is_ok_and(|v| v != "0"),
        shell: Shell::Plain,
        width: 0,
        duration_ms: None,
        context_percent: env_percent("ANVIL_AGENT_CTX_PERCENT", 73),
        token_percent: env_percent("ANVIL_AGENT_TOKEN_PERCENT", 48),
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--transient" => a.transient = true,
            "--agent" => a.agent = true,
            "--exit" => {
                if let Some(v) = it.next() {
                    a.exit_code = v.parse().unwrap_or(0);
                }
            }
            "--width" => {
                if let Some(v) = it.next() {
                    a.width = v.parse().unwrap_or(0);
                }
            }
            "--duration-ms" => {
                if let Some(v) = it.next() {
                    a.duration_ms = v.parse().ok();
                }
            }
            "--ctx-percent" => {
                if let Some(v) = it.next() {
                    a.context_percent = parse_percent(&v, a.context_percent);
                }
            }
            "--token-percent" => {
                if let Some(v) = it.next() {
                    a.token_percent = parse_percent(&v, a.token_percent);
                }
            }
            "--shell" => {
                if let Some(v) = it.next() {
                    a.shell = match v.as_str() {
                        "zsh" => Shell::Zsh,
                        "bash" => Shell::Bash,
                        _ => Shell::Plain,
                    };
                }
            }
            _ => {}
        }
    }
    a
}

fn env_percent(name: &str, default: u8) -> u8 {
    std::env::var(name)
        .ok()
        .map(|v| parse_percent(&v, default))
        .unwrap_or(default)
}

fn parse_percent(value: &str, default: u8) -> u8 {
    value.parse::<u8>().map(|v| v.min(100)).unwrap_or(default)
}

fn basename(path: &std::path::Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

fn main() {
    let args = parse_args();
    // Rich glyphs only inside Anvil.
    let rich = std::env::var("ANVIL").is_ok();

    if args.agent {
        print!(
            "{}",
            render::agent(AgentOptions {
                shell: args.shell,
                context_percent: args.context_percent,
                token_percent: args.token_percent,
            })
        );
        return;
    }

    if args.transient {
        let opts = Options {
            rich,
            failed: args.exit_code != 0,
            shell: args.shell,
            width: 0,
            duration_ms: None,
            git_dirty: 0,
            exit_code: args.exit_code,
        };
        print!("{}", render::transient(opts));
        return;
    }

    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(_) => return,
    };

    let context = detect(&cwd);
    let git_info = if context.in_git { query(&cwd) } else { None };

    // Dirty count comes from the git query already run — no extra subprocess.
    let git_dirty = git_info.as_ref().map(|g| g.dirty).unwrap_or(0);

    let cwd_base = basename(&cwd).to_string();
    let _list = assemble(Inputs {
        cwd_base: &cwd_base,
        context,
        git_info,
        exit_code: args.exit_code,
    });

    let opts = Options {
        rich,
        failed: args.exit_code != 0,
        shell: args.shell,
        width: args.width,
        duration_ms: args.duration_ms,
        git_dirty,
        exit_code: args.exit_code,
    };

    print!("{}", render::full(opts));
}
