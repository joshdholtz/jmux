#!/bin/sh
# jmux project status pane — refreshes every 30s

while true; do
    stty -echo 2>/dev/null
    clear

    jmux header "jmux"

    BRANCH=$(git branch --show-current 2>/dev/null || echo "unknown")
    AHEAD=$(git rev-list --count @{u}..HEAD 2>/dev/null || echo "0")
    DIRTY=$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')

    printf "\n"
    printf "  branch   %s\n" "$BRANCH"
    printf "  ahead    %s commit(s)\n" "$AHEAD"
    printf "  changes  %s file(s)\n" "$DIRTY"

    printf "\n"
    jmux header "Recent Commits"
    git log --oneline -5 2>/dev/null | while read -r line; do
        printf "  %s\n" "$line"
    done

    printf "\n"
    jmux header "Open PRs"
    if command -v gh >/dev/null 2>&1; then
        gh pr list --limit 20 --json number,title,isDraft \
            --template '{{range .}}{{if not .isDraft}}#{{.number}}  {{.title}}{{"\n"}}{{end}}{{end}}' 2>/dev/null \
            | jmux select --on-enter "gh pr view {1} --web" \
                          --empty-message "no open PRs" \
                          --timeout 30
    else
        printf "  (gh not installed)\n"
        sleep 30
    fi
done
