# Caldera Console — zsh ZDOTDIR shim.
# zsh reads $ZDOTDIR/.zshenv first. Restore the real ZDOTDIR so the rest of
# zsh startup (.zprofile/.zshrc/.zlogin) reads the user's own files, run the
# user's real .zshenv, then load the Caldera integration.

ZDOTDIR="${CALDERA_REAL_ZDOTDIR:-$HOME}"
unset CALDERA_REAL_ZDOTDIR

[ -f "$ZDOTDIR/.zshenv" ] && source "$ZDOTDIR/.zshenv"

[ -n "$CALDERA_SHELL_INTEGRATION_ZSH" ] && [ -r "$CALDERA_SHELL_INTEGRATION_ZSH" ] && \
  source "$CALDERA_SHELL_INTEGRATION_ZSH"
