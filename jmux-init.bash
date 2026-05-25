# jmux shell integration — add to ~/.bashrc:
#   eval "$(jmux init bash)"

__jmux_chpwd() {
    [[ -n "$JMUX_SOCKET" ]] || return
    jmux set-cwd "$PWD" 2>/dev/null &
}

__jmux_preexec() {
    [[ -n "$JMUX_SOCKET" ]] || return
    local cmd="${BASH_COMMAND%% *}"
    [[ "$cmd" == "__jmux"* || "$cmd" == "jmux" ]] && return
    jmux set-name "$cmd" 2>/dev/null &
}

trap '__jmux_preexec' DEBUG
PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND; }__jmux_chpwd"

j() {
    if [[ -z "$1" ]]; then
        jmux attach
    else
        jmux new "$1"
    fi
}
