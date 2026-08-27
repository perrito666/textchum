#!/usr/bin/env bash
# Builds a .deb from an already-built release binary.
#
#   cargo build --release --manifest-path linux/Cargo.toml
#   packaging/build-deb.sh [version]
#
# The version defaults to the current git tag. Everything lands in
# build/deb, and the finished package's path is printed at the end.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-$(git -C "$HERE" describe --tags --always 2>/dev/null || echo 0.0.0)}"
# Debian versions may not start with a letter; our tags are `v0.0.10`.
VERSION="${VERSION#v}"
BINARY="$HERE/linux/target/release/textchum-gtk"
[[ -x "$BINARY" ]] || {
    echo "no release binary at $BINARY — build it first" >&2
    exit 1
}

# dpkg's own name for this machine's architecture: amd64, arm64, …
ARCH="$(dpkg --print-architecture)"
STAGE="$HERE/build/deb/textchum_${VERSION}_${ARCH}"
rm -rf "$STAGE"

install -Dm755 "$BINARY" "$STAGE/usr/bin/textchum-gtk"
install -Dm755 "$HERE/scripts/chum" "$STAGE/usr/bin/chum"
install -Dm644 "$HERE/linux/data/to.perri.textchum.desktop" \
    "$STAGE/usr/share/applications/to.perri.textchum.desktop"
install -Dm644 "$HERE/packaging/to.perri.textchum.metainfo.xml" \
    "$STAGE/usr/share/metainfo/to.perri.textchum.metainfo.xml"
install -Dm644 "$HERE/linux/data/to.perri.textchum-512.png" \
    "$STAGE/usr/share/icons/hicolor/512x512/apps/to.perri.textchum.png"
install -Dm644 "$HERE/LICENSE" \
    "$STAGE/usr/share/doc/textchum/copyright"

mkdir -p "$STAGE/DEBIAN"
# Depends: the four libraries the shell links against, named as Debian
# names them. Recommends and Suggests rather than Depends for the rest:
# each switches off one feature and nothing else, which is the whole
# point of the editor degrading instead of refusing to start.
#
# Language servers and formatters appear nowhere. They are the user's
# own, in versions only they know, and a package manager has no business
# choosing them.
cat > "$STAGE/DEBIAN/control" <<CONTROL
Package: textchum
Version: $VERSION
Section: editors
Priority: optional
Architecture: $ARCH
Depends: \${shlibs:Depends}, libgtk-4-1, libadwaita-1-0, libgtksourceview-5-0, libwebkitgtk-6.0-4
Recommends: hunspell, hunspell-en-us
Suggests: universal-ctags, git
Maintainer: Horacio Duran <https://perri.to>
Homepage: https://perrito666.github.io/textchum/
Description: A native text editor with language servers and highlighting
 A text editor in the spirit of TextMate: native, fast, and focused on
 editing. One portable core in Rust owns every document; this is its
 GTK 4 and libadwaita shell.
 .
 Tree-sitter highlighting, language servers for diagnostics, completion
 and navigation, save preprocessors that run your own formatters, fuzzy
 file opening, project-wide regular-expression search, a live Markdown
 preview, and prose spell check that reads comments and leaves
 identifiers alone.
CONTROL

# The icon and desktop caches are per-system and stale after any
# install or removal that touches them.
cat > "$STAGE/DEBIAN/postinst" <<'POSTINST'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
    update-desktop-database -q /usr/share/applications 2>/dev/null || true
    gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
fi
POSTINST
cat > "$STAGE/DEBIAN/postrm" <<'POSTRM'
#!/bin/sh
set -e
if [ "$1" = remove ]; then
    update-desktop-database -q /usr/share/applications 2>/dev/null || true
    gtk-update-icon-cache -q /usr/share/icons/hicolor 2>/dev/null || true
fi
POSTRM
chmod 755 "$STAGE/DEBIAN/postinst" "$STAGE/DEBIAN/postrm"

# `${shlibs:Depends}` above is dh_shlibdeps' placeholder and means
# nothing to a plain dpkg-deb build; drop it rather than ship a
# dependency field with a literal substitution variable in it.
sed -i 's/\${shlibs:Depends}, //' "$STAGE/DEBIAN/control"

dpkg-deb --root-owner-group --build "$STAGE" >/dev/null
echo "$STAGE.deb"
