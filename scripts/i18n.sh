#!/usr/bin/env bash
# Extract the strings to translate and merge them into the catalogues.
#
#   scripts/i18n.sh              extract, then merge into every .po
#   scripts/i18n.sh --check      fail when a catalogue is out of date
#
# The source of truth is core/textchum-core/i18n/<language>.po, which is
# what translators and their tools speak. The build compiles each into
# the .mo the editor reads.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CATALOGUES="$ROOT/core/textchum-core/i18n"
POT="$CATALOGUES/textchum.pot"

command -v xgettext >/dev/null || {
    echo "xgettext is not installed (brew install gettext, apt install gettext)" >&2
    exit 1
}

cd "$ROOT"

# xgettext reads Rust and Swift as C, which is close enough to find the
# strings and far enough to misread a lifetime (`&'static`) or an
# apostrophe as an unterminated character. Those two complaints are
# noise; anything else it says is worth reading.
quiet() {
    grep -v "warning: unterminated \(character constant\|string literal\)" || true
}
# The lookups this codebase calls the catalogue by: tr/t for one string,
# tr_n/t_n/tn for the two forms of a plural.
xgettext \
    --keyword=tr --keyword=t --keyword=n_ \
    --keyword=tr_n:1,2 --keyword=t_n:1,2 --keyword=tn:1,2 \
    --from-code=UTF-8 --language=C --add-comments=TRANSLATORS \
    --package-name=textchum \
    --msgid-bugs-address=https://github.com/perrito666/textchum/issues \
    --output="$POT" \
    $(git ls-files '*.rs' '*.swift') 2> >(quiet >&2)

if [[ "${1:-}" == "--check" ]]; then
    status=0
    for po in "$CATALOGUES"/*.po; do
        missing=$(msgcmp --use-untranslated "$po" "$POT" 2>&1 | head -20 || true)
        if [[ -n "$missing" ]]; then
            echo "$(basename "$po") is behind the sources:" >&2
            echo "$missing" >&2
            status=1
        fi
    done
    exit $status
fi

for po in "$CATALOGUES"/*.po; do
    msgmerge --update --backup=none --quiet "$po" "$POT"
    msgfmt --check --statistics "$po" -o /dev/null
done
