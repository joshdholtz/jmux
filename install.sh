#!/usr/bin/env bash
# Run: bash install.sh
# Make executable first: chmod +x install.sh
set -e

echo "Installing jmux..."
cargo install --path . --locked

SHELL_NAME=$(basename "$SHELL")
RC_FILE=""
case "$SHELL_NAME" in
    zsh)  RC_FILE="$HOME/.zshrc" ;;
    bash) RC_FILE="$HOME/.bashrc" ;;
esac

if [[ -n "$RC_FILE" ]]; then
    INIT_LINE='eval "$(jmux init '"$SHELL_NAME"')"'
    if ! grep -q "jmux init" "$RC_FILE" 2>/dev/null; then
        echo "" >> "$RC_FILE"
        echo "# jmux shell integration" >> "$RC_FILE"
        echo "$INIT_LINE" >> "$RC_FILE"
        echo "Added jmux init to $RC_FILE"
    else
        echo "jmux init already in $RC_FILE — skipping"
    fi
fi

echo ""
echo "Done! Open a new shell or run:"
echo "  eval \"\$(jmux init $SHELL_NAME)\""
echo ""
echo "Then start jmux:"
echo "  jmux attach"
