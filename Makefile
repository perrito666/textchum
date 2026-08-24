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

.PHONY: all build core run test smoke header-check check docs docs-serve clean

all: build

## Rust core as a release static library (also regenerates textchum.h).
core:
	cargo build --release --manifest-path $(RUST_MANIFEST)

## Core + macOS app.
build: core
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

## Everything CI runs.
check: test smoke header-check

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
	rm -rf macos/.build site
