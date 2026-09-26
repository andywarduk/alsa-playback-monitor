#!/bin/bash
# Build and install alsa-playback-monitor system-wide with its systemd service.
#
#   ./install.sh             build, install, enable and (re)start the service
#   ./install.sh uninstall   stop and disable the service, remove the files
#
# Installs under $PREFIX (default /usr/local): the binary in $PREFIX/bin and
# the unit in $PREFIX/lib/systemd/system. Run as your normal user: cargo
# builds as you, and only the install steps use sudo.

set -euo pipefail

NAME=alsa-playback-monitor
PREFIX=${PREFIX:-/usr/local}
BIN=$PREFIX/bin/$NAME
UNIT=$PREFIX/lib/systemd/system/$NAME.service

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
    sudo systemctl daemon-reload
    sudo systemctl enable "$NAME"
    sudo systemctl restart "$NAME"
    systemctl --no-pager status "$NAME"
    ;;
uninstall)
    sudo systemctl disable --now "$NAME" || true
    sudo rm -f "$BIN" "$UNIT"
    sudo systemctl daemon-reload
    ;;
*)
    echo "usage: $0 [install|uninstall]" >&2
    exit 2
    ;;
esac
