#!/usr/bin/env bash
# Build a Fedora RPM from the existing `target/release/NekoNetease` binary.
#
# Run from anywhere; the script resolves the repo root via `git rev-parse`.
# Output: ~/rpmbuild/RPMS/<arch>/nekonetease-<ver>-1.fc<rel>.<arch>.rpm
#
# Usage:
#   ./scripts/build-rpm.sh                # uses existing release binary
#   ./scripts/build-rpm.sh --build        # cargo build --release first
#   ARCH=aarch64 ./scripts/build-rpm.sh   # aarch64 (default: native uname -m)
#   sudo dnf install ~/rpmbuild/RPMS/x86_64/nekonetease-*.rpm
#
# The RPM packages the prebuilt binary + a .desktop file + AppStream
# MetaInfo + the PNG logo + the project LICENSE + the third-party
# license texts + the polkit update helper/policy. Per-user data
# (queue.json, settings.json, nekonetease.db, artwork/) stays at
# `$XDG_DATA_HOME/NekoNetease/` and is shared across installs — install +
# uninstall doesn't touch user data.

set -euo pipefail

REPO_ROOT="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
cd "$REPO_ROOT"

if [[ "${1:-}" == "--build" ]]; then
  # `-p nekonetease` because the root is virtual: a bare build would compile every
  # member, testkit included, to produce the one binary packaged below.
  echo "==> cargo build --release -p nekonetease"
  cargo build --release -p nekonetease
fi

BINARY="${BINARY:-$REPO_ROOT/target/release/NekoNetease}"
[[ -f "$BINARY" ]] || { echo "ERROR: $BINARY not found. Run with --build first."; exit 1; }

# Every member inherits the version from `[workspace.package]`, so `[package]`
# reads `version.workspace = true` and carries no literal. Anchor on the table
# rather than taking the file's first `version = ` line.
VERSION="$(awk -F'"' '
  /^\[/                  { in_ws = ($0 == "[workspace.package]") }
  in_ws && /^version = / { print $2; exit }
' Cargo.toml)"
[[ -n "$VERSION" ]] || { echo "ERROR: no version in Cargo.toml's [workspace.package]"; exit 1; }
FEDORA_REL="$(rpm -E '%{?dist}' | sed 's/^\.//')"
ARCH="${ARCH:-$(uname -m)}"
case "$ARCH" in
  x86_64|aarch64) ;;
  *) echo "ERROR: unsupported ARCH=$ARCH (expected x86_64 or aarch64)"; exit 1 ;;
esac

echo "==> packaging nekonetease $VERSION for $FEDORA_REL ($ARCH)"

RPM_HOME="$HOME/rpmbuild"
mkdir -p "$RPM_HOME"/{SPECS,SOURCES,BUILD,BUILDROOT,RPMS,SRPMS}

# Build a source tarball that rpmbuild can unpack in %prep.
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
PKG_DIR="$STAGE/nekonetease-$VERSION"
mkdir -p "$PKG_DIR"

# Stage: binary + icon + desktop file + licenses + polkit helper/policy
cp "$BINARY" "$PKG_DIR/nekonetease"
chmod 0755 "$PKG_DIR/nekonetease"
# Supplied PNG artwork for the OS launcher / taskbar / KRunner icon and titlebar.
cp "$REPO_ROOT/assets/icons/nekonetease.png" "$PKG_DIR/nekonetease.png"
cp "$REPO_ROOT/LICENSE" "$PKG_DIR/LICENSE"
# The two bundled fonts, the vendored winit fork and the crates on
# copyleft terms are all compiled into the binary, so this package
# redistributes them and owes their license text — Apache-2.0 §4(a),
# GPLv3 §4 and MPL-2.0 §3.2 outright, and SIL's OFL FAQ
# recommends it for a bundled font even though the name-table metadata
# would technically do. `%license` below globs this directory and
# flattens the paths, so its contents land beside LICENSE in
# %{_licensedir}/nekonetease/ rather than in a `licenses/` subdirectory the
# way the DEB and AppImage lay them out. That still reads as
# licenses/ATTRIBUTION.txt describes it — "the full license text for
# each sits beside this file" — which is the only placement claim it
# makes, and the only one worth keeping true across five formats.
cp -r "$REPO_ROOT/licenses" "$PKG_DIR/licenses"

# Polkit helper + policy for branded auth prompts on in-app updater.
# The helper argv-dispatches to dnf5/dnf/apt/apt-get; the policy
# registers the helper as the target of action
# com.github.nekorain404.nekonetease.update so the KDE/GNOME auth dialog
# shows "Install NekoNetease update" instead of the raw
# `dnf install -y ...` command line.
cp "$REPO_ROOT/packaging/nekonetease-update-helper" "$PKG_DIR/nekonetease-update-helper"
chmod 0755 "$PKG_DIR/nekonetease-update-helper"
cp "$REPO_ROOT/packaging/com.github.nekorain404.nekonetease.update.policy" \
   "$PKG_DIR/com.github.nekorain404.nekonetease.update.policy"

# Desktop file named after the reverse-DNS app id so its desktop-id
# matches the AppStream component id `com.github.nekorain404.nekonetease` —
# software centres need that match to merge the .desktop entry with the
# MetaInfo component instead of generating a separate stub.
cat > "$PKG_DIR/com.github.nekorain404.nekonetease.desktop" <<'EOF'
[Desktop Entry]
Type=Application
Name=NekoNetease
GenericName=Music Player
Comment=Cross-platform desktop music player
Exec=nekonetease %F
Icon=nekonetease
Categories=AudioVideo;Audio;Player;Music;
Keywords=music;audio;player;library;sound;songs;tracks;mp3;flac;ogg;
Terminal=false
StartupWMClass=NekoNetease
MimeType=audio/mpeg;audio/flac;audio/x-flac;audio/mp4;audio/x-m4a;audio/x-m4b;audio/ogg;audio/x-vorbis+ogg;audio/x-flac+ogg;audio/x-opus+ogg;audio/wav;audio/x-wav;audio/aac;audio/x-aac;audio/aiff;audio/x-aiff;audio/x-aifc;audio/x-matroska;
EOF

# AppStream MetaInfo — KDE Discover / GNOME Software read this for the
# app's display name, developer and license. Shipped verbatim (no
# templating); the same file goes into the DEB, AppImage and per-user
# tarball paths too.
cp "$REPO_ROOT/packaging/com.github.nekorain404.nekonetease.metainfo.xml" \
   "$PKG_DIR/com.github.nekorain404.nekonetease.metainfo.xml"

# Best-effort validation — non-fatal so the build still works on hosts
# (e.g. CI runners) without the `appstream` package installed.
if command -v appstreamcli >/dev/null 2>&1; then
  appstreamcli validate --no-net "$PKG_DIR/com.github.nekorain404.nekonetease.metainfo.xml" \
    || echo "WARNING: appstreamcli reported MetaInfo issues (non-fatal)"
fi

(cd "$STAGE" && tar czf "$RPM_HOME/SOURCES/nekonetease-$VERSION.tar.gz" "nekonetease-$VERSION")

# Generate the SPEC.
cat > "$RPM_HOME/SPECS/nekonetease.spec" <<EOF
%global debug_package %{nil}

Name:           nekonetease
Version:        $VERSION
Release:        1%{?dist}
Summary:        Modern local-first music player with a Material 3-inspired interface
License:        AGPL-3.0-or-later
URL:            https://github.com/NekoRain404/NekoNetease
Packager:       NekoRain
Source0:        %{name}-%{version}.tar.gz

ExclusiveArch:  $ARCH

# Auto-detected runtime deps via rpmbuild's find-requires (fontconfig,
# freetype, wayland, libxkbcommon, alsa-lib, libGL, ...) cover
# everything the binary loads via dlopen / NEEDED entries — no manual
# Requires: list needed.

%description
NekoNetease is a modern, local-first desktop music player with a Material
3-inspired interface. Your music collection stays entirely on your own
machine, with no accounts, streaming or cloud.

It pairs a Slint user interface with a pure-Rust backend: gapless
playback via Symphonia and cpal (MP3, FLAC, M4A/M4B with AAC and ALAC,
Ogg Vorbis, Opus, WAV, AIFF/AIFF-C, Matroska and CAF), a fast local SQLite
library with full-text search, browsing by album, artist, genre and
playlist, OS media controls (MPRIS), and pluggable themes including
system light/dark and Material You.

%prep
%setup -q

%build
# binary is prebuilt; nothing to do

%install
install -D -m 0755 nekonetease %{buildroot}%{_bindir}/nekonetease
install -D -m 0644 com.github.nekorain404.nekonetease.desktop %{buildroot}%{_datadir}/applications/com.github.nekorain404.nekonetease.desktop
install -D -m 0644 com.github.nekorain404.nekonetease.metainfo.xml %{buildroot}%{_datadir}/metainfo/com.github.nekorain404.nekonetease.metainfo.xml
install -D -m 0644 nekonetease.png %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/nekonetease.png
install -D -m 0755 nekonetease-update-helper %{buildroot}%{_libexecdir}/nekonetease-update-helper
install -D -m 0644 com.github.nekorain404.nekonetease.update.policy %{buildroot}%{_datadir}/polkit-1/actions/com.github.nekorain404.nekonetease.update.policy

%post
update-desktop-database -q %{_datadir}/applications &>/dev/null || :
touch -c %{_datadir}/icons/hicolor &>/dev/null || :
gtk-update-icon-cache -q -t %{_datadir}/icons/hicolor &>/dev/null || :

%postun
if [ \$1 -eq 0 ]; then
    update-desktop-database -q %{_datadir}/applications &>/dev/null || :
    touch -c %{_datadir}/icons/hicolor &>/dev/null || :
    gtk-update-icon-cache -q -t %{_datadir}/icons/hicolor &>/dev/null || :
fi

%files
%license LICENSE licenses/*
%{_bindir}/nekonetease
%{_datadir}/applications/com.github.nekorain404.nekonetease.desktop
%{_datadir}/metainfo/com.github.nekorain404.nekonetease.metainfo.xml
%{_datadir}/icons/hicolor/256x256/apps/nekonetease.png
%{_libexecdir}/nekonetease-update-helper
%{_datadir}/polkit-1/actions/com.github.nekorain404.nekonetease.update.policy

%changelog
* $(date '+%a %b %d %Y') NekoRain - $VERSION-1
- Initial package of NekoNetease $VERSION.
EOF

echo "==> rpmbuild -bb --target $ARCH $RPM_HOME/SPECS/nekonetease.spec"
rpmbuild -bb --target "$ARCH" "$RPM_HOME/SPECS/nekonetease.spec"

# `%{?dist}` expands to empty on hosts without a Fedora dist tag (e.g.
# the Ubuntu CI runners), so the resulting filename has no `.fcXX` in
# the middle: `nekonetease-0.1.0-1.aarch64.rpm`, not
# `nekonetease-0.1.0-1.fc40.aarch64.rpm`. Match that shape with `${VAR:+...}`
# so the leading `.` only appears when there's actually a dist tag.
OUTPUT="$RPM_HOME/RPMS/$ARCH/nekonetease-$VERSION-1${FEDORA_REL:+.$FEDORA_REL}.$ARCH.rpm"
if [[ -f "$OUTPUT" ]]; then
  echo
  echo "==> built: $OUTPUT"
  echo "==> install with:"
  echo "    sudo dnf install $OUTPUT"
  echo "==> uninstall with:"
  echo "    sudo dnf remove nekonetease"
  echo
  ls -lh "$OUTPUT"
else
  echo "ERROR: expected $OUTPUT not found"
  ls -la "$RPM_HOME/RPMS/$ARCH/" 2>&1
  exit 1
fi
