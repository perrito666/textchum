#!/usr/bin/env bash
# Drives the built app with real input and checks it behaves as
# INTERACTION.md says.
#
#   make feel
#
# The smoke tests call the code directly, so they never see what only
# happens when events come the way a person's do: a balloon that takes
# the keyboard, a click swallowed on its way to the text, Return doing
# nothing in a list. This opens dist/Textchum.app on a small fixture,
# with a profile of its own and the scripted language server, and
# scripts/feel.js clicks, rests the pointer, double-clicks and types
# at it, reading back where the caret is and who has the keyboard.
#
# It takes over the screen for half a minute: do not touch the mouse or
# the keyboard while it runs. macOS has to let the terminal control the
# computer (System Settings ▸ Privacy & Security ▸ Accessibility). It is
# not part of `make check`: CI has no screen to drive.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
APP="$ROOT/dist/Textchum.app/Contents/MacOS/Textchum"
# Outside the repository: inside it, the outermost git root is the
# project, and the fixture's server would serve the whole checkout.
BASE="${TEXTCHUM_FEEL:-${TMPDIR:-/tmp}/textchum-feel}"
PROJECT="$BASE/project"
PROFILE="$BASE/profile"

[[ -x "$APP" ]] || { echo "feel: build the app first (make app)" >&2; exit 2; }
if pgrep -xq Textchum; then
    echo "feel: a Textchum is running; quit it first, so the keys go to the one under test" >&2
    exit 2
fi

rm -rf "$BASE"
mkdir -p "$PROJECT" "$PROFILE"
# The first line's first four letters carry the scripted server's
# finding, and the fourth line is where the caret is put.
cat > "$PROJECT/main.rs" <<'RUST'
make_thing();
fn main() {
    let total = 1;
    println!("{total}");
}
RUST
: > "$PROJECT/Cargo.toml"
git -C "$PROJECT" init -q
# The scripted server for Rust, and a font the driver can measure.
cat > "$PROFILE/config.json" <<JSON
{
  "lsp": {
    "servers": {"fake": {"command": "python3", "args": ["$ROOT/scripts/fake_lsp.py"], "languages": ["rust"]}},
    "defaults": {"rust": "fake"}
  },
  "editor": {"font_family": "Menlo", "font_size": 13}
}
JSON

"$APP" --data-dir "$PROFILE" --fresh "$PROJECT/main.rs" >"$BASE/app.log" 2>&1 &
APP_PID=$!
trap 'kill "$APP_PID" 2>/dev/null || true' EXIT
# The window, then the server's first answer.
for _ in $(seq 1 40); do
    osascript -e 'tell application "System Events" to count windows of process "Textchum"' 2>/dev/null | grep -qv '^0$' && break
    sleep 0.5
done
sleep 3

REPORT="$(osascript -l JavaScript "$HERE/feel.js")"
echo "$REPORT"
if grep -q '^FAIL' <<<"$REPORT"; then
    echo "feel: $(grep -c '^FAIL' <<<"$REPORT") failed (the app's log is $BASE/app.log)"
    exit 1
fi
echo "feel: ok"
