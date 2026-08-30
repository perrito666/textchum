# Single entry point for building, testing and documenting Textchum.
# See docs/getting-started.md for what each target does.

# Keep C objects (tree-sitter grammars) on the same deployment target as
# the Swift app, or the linker warns on every build.
export MACOSX_DEPLOYMENT_TARGET := 14.0

RUST_MANIFEST := core/Cargo.toml
CORE_LIB      := core/target/release/libtextchum.a
SWIFT_PKG     := --package-path macos
# Outside docs/ so MkDocs does not scan it as content.
DOCS_VENV     := .docs-venv

# `linux` MUST stay in this list: a directory of that name exists, and
# without .PHONY make declares it up to date and builds nothing.
.PHONY: all build core run test smoke header-check i18n-check check app playground docs docs-serve clean install-cli linux install-linux deb rpm

all: build

## Rust core as a release static library (also regenerates textchum.h).
core:
	cargo build --release --manifest-path $(RUST_MANIFEST)

## Core + macOS app. SwiftPM does not track the Rust static library as a
## dependency, so force the relink or a fresh core goes unlinked.
build: core
	rm -f macos/.build/debug/Textchum
	swift build $(SWIFT_PKG)

## Build everything and launch the editor.
run: core
	swift run $(SWIFT_PKG) Textchum

## Rust test suite.
test:
	cargo test --manifest-path $(RUST_MANIFEST)

## Headless end-to-end check of the Swift <-> core round trip.
smoke: build
	macos/.build/debug/Textchum --smoke-test

## Fail if a build left the committed C header out of date.
header-check: core
	git diff --exit-code macos/Sources/CTextchum/include/textchum.h

## Fail when the catalogues are behind the sources. Needs GNU gettext.
i18n-check:
	scripts/i18n.sh --check

## Everything CI runs.
check: test smoke header-check i18n-check

APP_BUNDLE := dist/Textchum.app

## A double-clickable application bundle in dist/, with the icon.
app: core
	rm -f macos/.build/release/Textchum
	swift build -c release $(SWIFT_PKG)
	rm -rf $(APP_BUNDLE)
	mkdir -p $(APP_BUNDLE)/Contents/MacOS $(APP_BUNDLE)/Contents/Resources
	cp macos/.build/release/Textchum $(APP_BUNDLE)/Contents/MacOS/
	cp macos/Info.plist $(APP_BUNDLE)/Contents/
	plutil -replace CFBundleShortVersionString \
	  -string "$$(git describe --tags --always --dirty 2>/dev/null || echo development)" \
	  $(APP_BUNDLE)/Contents/Info.plist
	cp scripts/chum $(APP_BUNDLE)/Contents/Resources/
	rm -rf dist/Textchum.iconset
	mkdir -p dist/Textchum.iconset
	for size in 16 32 128 256 512; do \
	  sips -z $$size $$size macos/AppIcon/icon-1024.png \
	    --out dist/Textchum.iconset/icon_$${size}x$${size}.png >/dev/null; \
	  sips -z $$((size*2)) $$((size*2)) macos/AppIcon/icon-1024.png \
	    --out dist/Textchum.iconset/icon_$${size}x$${size}@2x.png >/dev/null; \
	done
	iconutil -c icns dist/Textchum.iconset -o $(APP_BUNDLE)/Contents/Resources/Textchum.icns
	rm -rf dist/Textchum.iconset
	@echo "Built $(APP_BUNDLE) — open it, or copy to /Applications"

## A throwaway project and profile to develop against: a git repository
## with Python and Rust, a remote that goes nowhere, uncommitted edits,
## and a scratch profile so the real one is never opened. KEEP=1 reuses
## the one already made.
playground:
	scripts/playground.sh

## The GTK shell (Linux only; needs gtk4/libadwaita/gtksourceview dev
## packages — see docs/getting-started.md).
linux:
	cargo build --release --manifest-path linux/Cargo.toml

## Installs the Linux build for the current user: binary, desktop
## entry, and icon into the XDG home directories.
install-linux: linux
	install -Dm755 linux/target/release/textchum-gtk $(HOME)/.local/bin/textchum-gtk
	install -Dm644 linux/data/to.perri.textchum.desktop $(HOME)/.local/share/applications/to.perri.textchum.desktop
	install -Dm644 macos/AppIcon/icon-1024.png $(HOME)/.local/share/icons/hicolor/1024x1024/apps/to.perri.textchum.png
	install -Dm644 linux/data/to.perri.textchum-512.png $(HOME)/.local/share/icons/hicolor/512x512/apps/to.perri.textchum.png
	test -f $(HOME)/.local/share/icons/hicolor/index.theme || \
		test ! -f /usr/share/icons/hicolor/index.theme || \
		install -Dm644 /usr/share/icons/hicolor/index.theme $(HOME)/.local/share/icons/hicolor/index.theme
	-gtk-update-icon-cache $(HOME)/.local/share/icons/hicolor 2>/dev/null
	-update-desktop-database $(HOME)/.local/share/applications 2>/dev/null
	@echo Installed textchum-gtk for $(USER) — ensure ~/.local/bin is on PATH

## A .deb from the release binary, into build/deb.
deb: linux
	packaging/build-deb.sh

## An .rpm, built from source by rpmbuild, into build/rpm.
rpm:
	packaging/build-rpm.sh

PREFIX ?= /usr/local

## Installs the `chum` terminal command (chum [+LINE] [-t|-w] file...).
install-cli:
	install -d $(PREFIX)/bin
	install -m 0755 scripts/chum $(PREFIX)/bin/chum
	@echo "Installed $(PREFIX)/bin/chum"

# Newest available Python: patched versions of the docs toolchain's
# transitive dependencies require >= 3.10.
PYTHON := $(shell command -v python3.13 || command -v python3.12 || command -v python3.11 || command -v python3)

$(DOCS_VENV): docs/requirements.txt
	$(PYTHON) -m venv $(DOCS_VENV)
	$(DOCS_VENV)/bin/pip install --quiet -r docs/requirements.txt
	touch $(DOCS_VENV)

## Static documentation site (en/es/fr) into site/.
docs: $(DOCS_VENV)
	$(DOCS_VENV)/bin/mkdocs build --strict

## Live-reloading documentation preview.
docs-serve: $(DOCS_VENV)
	$(DOCS_VENV)/bin/mkdocs serve

clean:
	cargo clean --manifest-path $(RUST_MANIFEST)
	rm -rf macos/.build site dist
