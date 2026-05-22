//! anvil-prompt — renders the Anvil shell prompt. Invoked by the shell on
//! every prompt draw. Args: --exit <n>, --transient, --shell <zsh|bash|plain>,
//! --width <n> (accepted for forward-compat, unused). Emits ANSI to stdout.

use anvil_prompt_core::build_segments::{Inputs, assemble};
use anvil_prompt_core::context::detect;
use anvil_prompt_core::git::query;
use anvil_prompt_core::render::{self, Options, Shell};

struct Args {
    exit_code: u8,
    transient: bool,
    shell: Shell,
}

fn parse_args() -> Args {
    let mut a = Args {
        exit_code: 0,
        transient: false,
        shell: Shell::Plain,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--transient" => a.transient = true,
            "--exit" => {
                if let Some(v) = it.next() {
                    a.exit_code = v.parse().unwrap_or(0);
                }
            }
            "--width" => {
                // accepted for forward-compat, unused
                let _ = it.next();
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

fn basename(path: &std::path::Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

fn main() {
    let args = parse_args();
    // Rich glyphs only inside Anvil.
    let rich = std::env::var("ANVIL").is_ok();
    let opts = Options {
        rich,
        failed: args.exit_code != 0,
        shell: args.shell,
    };

    if args.transient {
        print!("{}", render::transient(opts));
        return;
    }

    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(_) => return,
    };

    let context = detect(&cwd);
    let git_info = if context.in_git { query(&cwd) } else { None };

    let cwd_base = basename(&cwd).to_string();
    let list = assemble(Inputs {
        cwd_base: &cwd_base,
        context,
        git_info,
        exit_code: args.exit_code,
    });

    print!("{}", render::full(list.slice(), opts));
}
