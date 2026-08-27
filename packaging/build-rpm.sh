#!/usr/bin/env bash
# Builds an .rpm from the repository, using packaging/textchum.spec.
#
#   packaging/build-rpm.sh [version]
#
# Unlike the .deb script this builds from source inside rpmbuild, which
# is what an RPM is expected to do — the spec's %build runs cargo. The
# finished package's path is printed at the end.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-$(git -C "$HERE" describe --tags --always 2>/dev/null || echo 0.0.0)}"
VERSION="${VERSION#v}"

command -v rpmbuild >/dev/null || {
    echo "rpmbuild not found — install rpm (Debian) or rpm-build (Fedora)" >&2
    exit 1
}

TOP="$HERE/build/rpm"
rm -rf "$TOP"
mkdir -p "$TOP"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}

# The tarball has to unpack into name-version/, which %autosetup expects.
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/textchum-$VERSION"
if git -C "$HERE" rev-parse --git-dir >/dev/null 2>&1; then
    # Everything tracked, so the archive matches the repository rather
    # than whatever is lying in the working tree — including target/,
    # which is gigabytes.
    git -C "$HERE" archive HEAD | tar -x -C "$STAGE/textchum-$VERSION"
else
    # No repository: an unpacked release tarball, or a copy taken
    # without .git. Fall back to the tree, minus the build output that
    # `git archive` would have left out anyway.
    tar -c -C "$HERE" \
        --exclude=.git \
        --exclude=./build \
        --exclude=./target \
        --exclude=./core/target \
        --exclude=./linux/target \
        --exclude=./macos/.build \
        --exclude=./site \
        --exclude=./dist \
        --exclude=./.docs-venv \
        . | tar -x -C "$STAGE/textchum-$VERSION"
fi
tar -czf "$TOP/SOURCES/textchum-$VERSION.tar.gz" -C "$STAGE" "textchum-$VERSION"

cp "$HERE/packaging/textchum.spec" "$TOP/SPECS/"
# RPMBUILD_FLAGS=--nodeps is for building on a distribution whose rpm
# cannot see the libraries because they are installed as .debs. It says
# nothing about whether the BuildRequires are right — only that this
# machine cannot check them.
# shellcheck disable=SC2086
rpmbuild ${RPMBUILD_FLAGS:-} \
    --define "_topdir $TOP" \
    --define "version $VERSION" \
    -bb "$TOP/SPECS/textchum.spec"

find "$TOP/RPMS" -name '*.rpm' -print
