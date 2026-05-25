# jmux shell integration — add to ~/.zshrc:
#   eval "$(jmux init zsh)"

__jmux_chpwd() {
    [[ -n "$JMUX_SOCKET" ]] || return
    jmux set-cwd "$PWD" 2>/dev/null &!
}

__jmux_preexec() {
    [[ -n "$JMUX_SOCKET" ]] || return
    local cmd="${1%% *}"
    jmux set-name "$cmd" 2>/dev/null &!
}

autoload -Uz add-zsh-hook
add-zsh-hook chpwd __jmux_chpwd
add-zsh-hook preexec __jmux_preexec

j() {
    if [[ -z "$1" ]]; then
        jmux attach
    else
        jmux new "$1"
    fi
}
