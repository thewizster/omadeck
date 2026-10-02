#!/usr/bin/env bash
# Build and install omadeck for the current user (no root needed except for the
# optional udev rule). Re-run any time to upgrade.
#
#   PREFIX=~/.local scripts/install.sh       (default)
#   SKIP_BUILD=1 scripts/install.sh          (use an existing release build)
set -euo pipefail

cd "$(dirname "$0")/.."
PREFIX="${PREFIX:-$HOME/.local}"
BINDIR="$PREFIX/bin"
DATADIR="${XDG_DATA_HOME:-$HOME/.local/share}"
UNITDIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"

say() { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*"; }

if ! command -v cargo >/dev/null; then
  warn "cargo not found. Install Rust first, e.g.:  mise use -g rust@latest"
  exit 1
fi
for pkg in gtk4 libadwaita-1 libudev; do
  pkg-config --exists "$pkg" 2>/dev/null || { warn "missing $pkg — on Omarchy/Arch: sudo pacman -S --needed gtk4 libadwaita systemd-libs"; exit 1; }
done

if [[ -z ${SKIP_BUILD:-} ]]; then
  say "Building (release)…"
  # --locked: build exactly the dependency versions pinned in Cargo.lock, never newer ones.
  cargo build --release --locked
fi

say "Installing binaries to $BINDIR"
install -Dm755 target/release/omadeck "$BINDIR/omadeck"
install -Dm755 target/release/omadeckd "$BINDIR/omadeckd"

say "Installing desktop entry and icon"
install -Dm644 packaging/dev.omadeck.Omadeck.desktop "$DATADIR/applications/dev.omadeck.Omadeck.desktop"
sed -i "s|^Exec=omadeck$|Exec=$BINDIR/omadeck|" "$DATADIR/applications/dev.omadeck.Omadeck.desktop"
install -Dm644 assets/dev.omadeck.Omadeck.svg "$DATADIR/icons/hicolor/scalable/apps/dev.omadeck.Omadeck.svg"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$DATADIR/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$DATADIR/icons/hicolor" || true

say "Installing systemd user service"
mkdir -p "$UNITDIR"
sed "s|@BINDIR@|$BINDIR|" packaging/omadeckd.service >"$UNITDIR/omadeckd.service"
systemctl --user daemon-reload

# Device permissions: many systems already grant seat users access to the deck's
# hidraw node; only install the udev rule if we can't open it.
needs_rule=1
for dev in /sys/class/hidraw/hidraw*; do
  if grep -qi "HID_ID=.*:00000FD9:" "$dev/device/uevent" 2>/dev/null; then
    [[ -r /dev/$(basename "$dev") && -w /dev/$(basename "$dev") ]] && needs_rule=0
  fi
done
if [[ -f /etc/udev/rules.d/70-omadeck.rules ]]; then
  needs_rule=0
fi
if ((needs_rule)); then
  rule_cmds=(
    "sudo install -Dm644 packaging/70-omadeck.rules /etc/udev/rules.d/70-omadeck.rules"
    "sudo udevadm control --reload-rules"
    "sudo udevadm trigger --attr-match=idVendor=0fd9"
  )
  say "Your user can't open the Stream Deck yet. omadeck can install a udev rule that"
  say "grants the logged-in user access to Elgato devices only (vendor 0fd9):"
  echo
  sed 's/^/      /' packaging/70-omadeck.rules
  echo
  say "This runs, as root:"
  printf '      %s\n' "${rule_cmds[@]}"
  answer=n
  if [[ -t 0 ]]; then
    read -rp "    Run these now? [y/N] " answer
  fi
  if [[ $answer == [yY]* ]]; then
    sudo install -Dm644 packaging/70-omadeck.rules /etc/udev/rules.d/70-omadeck.rules &&
      sudo udevadm control --reload-rules &&
      sudo udevadm trigger --attr-match=idVendor=0fd9 ||
      warn "Installing the udev rule failed; run the commands above yourself."
  else
    warn "Skipped. Run the commands above yourself, then replug the deck."
  fi
else
  say "Stream Deck is already accessible; no udev rule needed"
fi

say "Starting omadeckd"
systemctl --user enable omadeckd.service >/dev/null 2>&1
systemctl --user restart omadeckd.service

say "Done! Open \"omadeck\" from your launcher (Super + Space) to set up your keys."
[[ ":$PATH:" == *":$BINDIR:"* ]] || warn "$BINDIR is not on your PATH"
