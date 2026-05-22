# Anvil — zsh shell integration.
# Emits OSC 133 semantic prompt marks and OSC 7 working-directory reports.
# Sourced by the Anvil ZDOTDIR shim; safe to source manually too.

[[ -n "$ANVIL_ZSH_LOADED" ]] && return
ANVIL_ZSH_LOADED=1

# precmd: the previous command finished (133;D + exit), a new prompt starts
# (133;A), and report the cwd (OSC 7).
__anvil_precmd() {
  local last=$?
  typeset -g ANVIL_EXIT=$last
  printf '\e]133;D;%s\a' "$last"
  printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "$PWD"
  printf '\e]133;A\a'
}

# preexec: a command is about to run (133;C).
__anvil_preexec() {
  printf '\e]133;C\a'
}

typeset -ag precmd_functions preexec_functions
precmd_functions+=(__anvil_precmd)
preexec_functions+=(__anvil_preexec)

# 133;B marks the end of the prompt / start of typed input. Append it to PS1
# as a zero-width segment. Done from a one-shot precmd so it runs *after* the
# user's .zshrc has set PS1, then removes itself.
__anvil_mark_prompt() {
  if [[ "$PS1" != *$'\e]133;B'* ]]; then
    PS1="${PS1}%{"$'\e]133;B\a'"%}"
  fi
  precmd_functions=(${precmd_functions:#__anvil_mark_prompt})
}
precmd_functions+=(__anvil_mark_prompt)

# Anvil prompt — when the binary is known, drive PROMPT from it.
if [[ -n "$ANVIL_PROMPT" && -x "$ANVIL_PROMPT" ]]; then
  setopt prompt_subst
  __anvil_prompt() {
    PROMPT="$("$ANVIL_PROMPT" --exit ${ANVIL_EXIT:-0} --width "${COLUMNS:-80}" --shell zsh 2>/dev/null)"
  }
  precmd_functions+=(__anvil_prompt)
fi
