#!/bin/bash
# Build and install alsa-playback-monitor system-wide with its systemd service.
#
#   ./install.sh             build, install, enable and (re)start the service
#   ./install.sh uninstall   stop and disable the service, remove the files
#
# Installs under $PREFIX (default /usr/local): the binary in $PREFIX/bin and
# the unit in $PREFIX/lib/systemd/system. The service's hook script,
# /etc/alsa-playback-monitor/hook.sh, is installed from examples/hook.sh only
# if it doesn't exist, so local edits survive reinstalls. Uninstalling removes
# it only if it still matches examples/hook.sh, i.e. it hasn't been edited.
# Run as your normal user: cargo builds as you, and only the install steps
# use sudo.

set -euo pipefail

NAME=alsa-playback-monitor
PREFIX=${PREFIX:-/usr/local}
BIN=$PREFIX/bin/$NAME
UNIT=$PREFIX/lib/systemd/system/$NAME.service
HOOK=/etc/$NAME/hook.sh

if [[ $EUID -eq 0 ]]; then
    echo "Run this as your normal user, not root; it uses sudo where needed." >&2
    exit 1
fi

cd "$(dirname "$0")"

case ${1:-install} in
install)
    cargo build --release
    sudo install -Dm755 "target/release/$NAME" "$BIN"
    sudo install -d "$(dirname "$UNIT")"
    sed "s|^ExecStart=/usr/local/bin/|ExecStart=$PREFIX/bin/|" "$NAME.service" | sudo tee "$UNIT" > /dev/null
    if [[ -e $HOOK ]]; then
        echo "Keeping existing $HOOK"
    else
        sudo install -Dm755 examples/hook.sh "$HOOK"
        echo "Installed $HOOK"
    fi
    sudo systemctl daemon-reload
    sudo systemctl enable "$NAME"
    sudo systemctl restart "$NAME"
    systemctl --no-pager status "$NAME"
    ;;
uninstall)
    sudo systemctl disable --now "$NAME" || true
    sudo rm -f "$BIN" "$UNIT"
    sudo systemctl daemon-reload
    if cmp -s examples/hook.sh "$HOOK"; then
        sudo rm "$HOOK"
        sudo rmdir --ignore-fail-on-non-empty "$(dirname "$HOOK")"
        echo "Removed unmodified $HOOK"
    elif [[ -e $HOOK ]]; then
        echo "Kept $HOOK as it differs from examples/hook.sh; delete /etc/$NAME yourself if it's no longer wanted."
    fi
    ;;
*)
    echo "usage: $0 [install|uninstall]" >&2
    exit 2
    ;;
esac
