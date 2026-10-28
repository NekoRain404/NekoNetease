#!/usr/bin/env bash
# Uninstall the per-user NekoNetease install created by install-linux.sh.
#
# Per-user data (queue.json, settings.json, nekonetease.db, artwork/) is
# left in place at $XDG_DATA_HOME/NekoNetease/ — that directory is shared
# with `dnf`-installed builds and contains the user's library, so we
# don't touch it. To wipe data too, follow the printed hint.
#
# Environment overrides:
#   NEKONETEASE_INSTALL_DIR   — install dir (default: ~/.local/share/NekoNetease)
#   XDG_DATA_HOME         — base for ~/.local/share

set -euo pipefail

DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
INSTALL_DIR="${NEKONETEASE_INSTALL_DIR:-$DATA_HOME/NekoNetease}"
APPS_DIR="$DATA_HOME/applications"
ICONS_DIR="$DATA_HOME/icons/hicolor/256x256/apps"

removed=0
remove() {
  if [[ -e "$1" ]]; then
    rm -f "$1"
    echo "  removed $1"
    removed=$((removed + 1))
  fi
}

# `remove` can't take the third-party licence dir — its contents are whatever
# `licenses/` held at package time, so removing them by name here would drift
# the moment a fourth file lands there.
remove_dir() {
  if [[ -d "$1" ]]; then
    rm -rf "$1"
    echo "  removed $1"
    removed=$((removed + 1))
  fi
}

echo "==> uninstalling NekoNetease from $INSTALL_DIR"
remove "$INSTALL_DIR/NekoNetease"
remove "$INSTALL_DIR/LICENSE"
remove_dir "$INSTALL_DIR/licenses"
# Try to rmdir the install dir; non-fatal if user data is mixed in.
[[ -d "$INSTALL_DIR" ]] && rmdir --ignore-fail-on-non-empty "$INSTALL_DIR" 2>/dev/null || true

remove "$APPS_DIR/com.github.helpmeeadice.nekonetease.desktop"
# Legacy: earlier per-user installs used `NekoNetease.desktop` (capital M)
# and then `nekonetease.desktop`. Mirrors the `NekoNetease.svg` cleanup below.
remove "$APPS_DIR/NekoNetease.desktop"
remove "$APPS_DIR/nekonetease.desktop"
# AppStream MetaInfo — NekoNetease self-deploys this to the per-user
# metainfo dir on first launch (see desktop_integration.rs).
remove "$DATA_HOME/metainfo/com.github.helpmeeadice.nekonetease.metainfo.xml"
remove "$ICONS_DIR/nekonetease.png"
# Clean the scalable SVG installed by previous releases too.
remove "$DATA_HOME/icons/hicolor/scalable/apps/nekonetease.svg"
remove "$DATA_HOME/icons/hicolor/scalable/apps/NekoNetease.svg"

# Symlink under ~/.local/bin/ — only created if the user opted in
# during install (mentioned in install script's tail). Best-effort.
# Only a link that points back into this install is ours to delete; the name is generic enough
# that another tool may have claimed it, and removing that link would break that tool.
LINK="$HOME/.local/bin/nekonetease"
if [[ -L "$LINK" ]]; then
  target="$(readlink -f "$LINK" 2>/dev/null || true)"
  case "$target" in
    "$INSTALL_DIR"/*) remove "$LINK" ;;
    *) echo "  left $LINK alone (points at $target)" ;;
  esac
fi

# Cache refresh.
if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database -q "$APPS_DIR" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -q -t "$DATA_HOME/icons/hicolor" || true
fi

if [[ "$removed" -eq 0 ]]; then
  echo "Nothing to remove — NekoNetease wasn't installed at $INSTALL_DIR."
  exit 0
fi

cat <<EOF

✓ Uninstalled $removed file(s).

User data (library, settings, queue) is preserved at:
    $DATA_HOME/NekoNetease/

To wipe everything including data, also run:
    rm -rf "$DATA_HOME/NekoNetease"
EOF
