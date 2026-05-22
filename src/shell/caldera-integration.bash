# Caldera Console — bash shell integration.
# Emits OSC 133 semantic prompt marks and OSC 7 working-directory reports.
# Opt-in: add to ~/.bashrc:
#   [ -n "$CALDERA_CONSOLE" ] && [ -r "$CALDERA_SHELL_INTEGRATION" ] && . "$CALDERA_SHELL_INTEGRATION"

[ -n "$CALDERA_BASH_LOADED" ] && return
CALDERA_BASH_LOADED=1

__caldera_precmd() {
  local last=$?
  printf '\e]133;D;%s\a' "$last"
  printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$PWD"
  printf '\e]133;A\a'
}

# DEBUG fires before every simple command; suppress it while PROMPT_COMMAND
# itself runs so only real commands emit 133;C.
__caldera_preexec() {
  [ -n "$__caldera_in_prompt" ] && return
  printf '\e]133;C\a'
}

__caldera_prompt_wrapper() {
  __caldera_in_prompt=1
  __caldera_precmd
  unset __caldera_in_prompt
}

PROMPT_COMMAND="__caldera_prompt_wrapper${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
trap '__caldera_preexec' DEBUG

case "$PS1" in
  *'133;B'*) ;;
  *) PS1="${PS1}\[\e]133;B\a\]" ;;
esac

# Caldera prompt — bash gets the full prompt each draw (no transient collapse).
if [[ -n "$CALDERA_PROMPT" && -x "$CALDERA_PROMPT" ]]; then
  __caldera_prompt() {
    PS1="$("$CALDERA_PROMPT" --exit $? 2>/dev/null)"
  }
  PROMPT_COMMAND="__caldera_prompt${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
fi
