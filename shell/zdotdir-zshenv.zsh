# Anvil — zsh ZDOTDIR shim.
# zsh reads $ZDOTDIR/.zshenv first. Restore the real ZDOTDIR so the rest of
# zsh startup (.zprofile/.zshrc/.zlogin) reads the user's own files, run the
# user's real .zshenv, then load the Anvil integration.

ZDOTDIR="${ANVIL_REAL_ZDOTDIR:-$HOME}"
unset ANVIL_REAL_ZDOTDIR

[ -f "$ZDOTDIR/.zshenv" ] && source "$ZDOTDIR/.zshenv"

[ -n "$ANVIL_SHELL_INTEGRATION_ZSH" ] && [ -r "$ANVIL_SHELL_INTEGRATION_ZSH" ] && \
  source "$ANVIL_SHELL_INTEGRATION_ZSH"
