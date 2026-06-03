// Shell-integration snippets that emit OSC 133 prompt marks, which drive the
// terminal's command separators, command blocks, and exit-status capture.
// The user adds one of these to their shell rc (we offer a copy-to-clipboard
// command); we deliberately don't mutate their rc files automatically.

// zsh: emit D (previous exit) + A (prompt start) before each prompt.
export const ZSH_INTEGRATION = `# Anvil shell integration (OSC 133) — command separators + blocks
autoload -Uz add-zsh-hook 2>/dev/null
__anvil_precmd() { printf '\\033]133;D;%s\\007\\033]133;A\\007' "$?"; }
add-zsh-hook precmd __anvil_precmd 2>/dev/null`;

// bash: same, appended to PROMPT_COMMAND (idempotent).
export const BASH_INTEGRATION = `# Anvil shell integration (OSC 133) — command separators + blocks
__anvil_precmd() { local e=$?; printf '\\033]133;D;%s\\007\\033]133;A\\007' "$e"; }
case "$PROMPT_COMMAND" in *__anvil_precmd*) ;; *) PROMPT_COMMAND="__anvil_precmd\${PROMPT_COMMAND:+;$PROMPT_COMMAND}";; esac`;

// fish: hook the prompt event.
export const FISH_INTEGRATION = `# Anvil shell integration (OSC 133) — command separators + blocks
function __anvil_precmd --on-event fish_prompt
    printf '\\033]133;D;%s\\007\\033]133;A\\007' $status
end`;

export type IntegrationShell = "zsh" | "bash" | "fish";

const RC: Record<IntegrationShell, string> = { zsh: "~/.zshrc", bash: "~/.bashrc", fish: "~/.config/fish/config.fish" };

/** Snippet for a shell, picked from a shell path/name (defaults to zsh). */
export function integrationFor(shell: string): { snippet: string; rc: string; shell: IntegrationShell } {
  const s = (shell || "").toLowerCase();
  const kind: IntegrationShell = s.includes("fish") ? "fish" : s.includes("bash") ? "bash" : "zsh";
  const snippet = kind === "fish" ? FISH_INTEGRATION : kind === "bash" ? BASH_INTEGRATION : ZSH_INTEGRATION;
  return { snippet, rc: RC[kind], shell: kind };
}
