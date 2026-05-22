# Anvil — bash shell integration.
# Emits OSC 133 semantic prompt marks and OSC 7 working-directory reports.
# Opt-in: add to ~/.bashrc:
#   [ -n "$ANVIL" ] && [ -r "$ANVIL_SHELL_INTEGRATION" ] && . "$ANVIL_SHELL_INTEGRATION"

[ -n "$ANVIL_BASH_LOADED" ] && return
ANVIL_BASH_LOADED=1

__anvil_precmd() {
  local last=$?
  printf '\e]133;D;%s\a' "$last"
  printf '\e]7;file://%s%s\a' "${HOSTNAME:-localhost}" "$PWD"
  printf '\e]133;A\a'
}

# DEBUG fires before every simple command; suppress it while PROMPT_COMMAND
# itself runs so only real commands emit 133;C.
__anvil_preexec() {
  [ -n "$__anvil_in_prompt" ] && return
  printf '\e]133;C\a'
}

__anvil_prompt_wrapper() {
  __anvil_in_prompt=1
  __anvil_precmd
  unset __anvil_in_prompt
}

PROMPT_COMMAND="__anvil_prompt_wrapper${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
trap '__anvil_preexec' DEBUG

case "$PS1" in
  *'133;B'*) ;;
  *) PS1="${PS1}\[\e]133;B\a\]" ;;
esac

# Anvil prompt — bash gets the full prompt each draw (no transient collapse).
if [[ -n "$ANVIL_PROMPT" && -x "$ANVIL_PROMPT" ]]; then
  __anvil_prompt() {
    local last=$?
    PS1="$("$ANVIL_PROMPT" --exit "$last" --width "${COLUMNS:-80}" --shell bash 2>/dev/null)"
  }
  PROMPT_COMMAND="__anvil_prompt${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
fi
