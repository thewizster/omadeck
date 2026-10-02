#!/usr/bin/env bash
# Remove omadeck. Your configuration in ~/.config/omadeck is left alone.
set -euo pipefail
PREFIX="${PREFIX:-$HOME/.local}"
DATADIR="${XDG_DATA_HOME:-$HOME/.local/share}"
UNITDIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

systemctl --user disable --now omadeckd.service 2>/dev/null || true
rm -f "$UNITDIR/omadeckd.service"
systemctl --user daemon-reload
rm -f "$PREFIX/bin/omadeck" "$PREFIX/bin/omadeckd"
rm -f "$DATADIR/applications/dev.omadeck.Omadeck.desktop"
rm -f "$DATADIR/icons/hicolor/scalable/apps/dev.omadeck.Omadeck.svg"
if [[ -f /etc/udev/rules.d/70-omadeck.rules ]]; then
  echo "Removing udev rule (needs sudo)"
  sudo rm -f /etc/udev/rules.d/70-omadeck.rules && sudo udevadm control --reload-rules || true
fi
echo "omadeck removed. Your config is still in ${XDG_CONFIG_HOME:-$HOME/.config}/omadeck"
