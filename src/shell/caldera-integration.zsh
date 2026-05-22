# Caldera Console — zsh shell integration.
# Emits OSC 133 semantic prompt marks and OSC 7 working-directory reports.
# Sourced by the Caldera ZDOTDIR shim; safe to source manually too.

[[ -n "$CALDERA_ZSH_LOADED" ]] && return
CALDERA_ZSH_LOADED=1

# precmd: the previous command finished (133;D + exit), a new prompt starts
# (133;A), and report the cwd (OSC 7).
__caldera_precmd() {
  local last=$?
  printf '\e]133;D;%s\a' "$last"
  printf '\e]7;file://%s%s\a' "${HOST:-localhost}" "$PWD"
  printf '\e]133;A\a'
}

# preexec: a command is about to run (133;C).
__caldera_preexec() {
  printf '\e]133;C\a'
}

typeset -ag precmd_functions preexec_functions
precmd_functions+=(__caldera_precmd)
preexec_functions+=(__caldera_preexec)

# 133;B marks the end of the prompt / start of typed input. Append it to PS1
# as a zero-width segment. Done from a one-shot precmd so it runs *after* the
# user's .zshrc has set PS1, then removes itself.
__caldera_mark_prompt() {
  if [[ "$PS1" != *$'\e]133;B'* ]]; then
    PS1="${PS1}%{"$'\e]133;B\a'"%}"
  fi
  precmd_functions=(${precmd_functions:#__caldera_mark_prompt})
}
precmd_functions+=(__caldera_mark_prompt)

# Caldera prompt — when the binary is known, drive PROMPT from it.
if [[ -n "$CALDERA_PROMPT" && -x "$CALDERA_PROMPT" ]]; then
  setopt prompt_subst
  __caldera_prompt() {
    PROMPT="$("$CALDERA_PROMPT" --exit $? 2>/dev/null)"
  }
  precmd_functions+=(__caldera_prompt)

  # Transient: on accept-line, redraw the finished prompt collapsed.
  __caldera_transient() {
    PROMPT="$("$CALDERA_PROMPT" --transient 2>/dev/null)"
    zle .reset-prompt
  }
  zle -N zle-line-finish __caldera_transient
fi
