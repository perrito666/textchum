#!/usr/bin/env bash
# Photograph the guest's screen, or one window of it.
#
#   ./vm-shot.sh out.png                         the whole screen
#   ./vm-shot.sh out.png "Widget Factory"        just that window
#   ./vm-shot.sh out.png "Widget Factory" 1100x740   at a set size
#   ./vm-shot.sh out.png --host                  UTM's window, from the host
#
# The picture is taken inside the guest and pulled back, which is what
# makes single-window capture possible — from the host you can only ever
# get the whole guest screen, GNOME top bar and all.
#
# This needs the session on Xorg, which is what provision.sh sets up.
# Under Wayland none of it works: GNOME's Shell.Screenshot answers
# "Screenshot is not allowed" to anything that is not an interactive
# user action, and grim does nothing under Mutter, which implements none
# of the wlroots screencopy protocol it needs. Use --host when you have
# deliberately switched the session to Wayland.
set -euo pipefail

VM_NAME="${VM_NAME:-textchum-linux}"
VM_USER="${VM_USER:-textchum}"
UTMCTL="/Applications/UTM.app/Contents/MacOS/utmctl"

OUT="${1:-}"
TARGET="${2:-}"
SIZE="${3:-}"
[[ -n "$OUT" ]] || {
    echo "usage: $(basename "$0") <out.png> [window name|--host] [WxH]" >&2; exit 2
}

if [[ "$TARGET" == "--host" ]]; then
    WID="$(swift - "$VM_NAME" <<'SWIFT'
import CoreGraphics
import Foundation
let wanted = CommandLine.arguments[1]
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID)
    as! [[String: Any]]
for window in windows where (window["kCGWindowName"] as? String) == wanted {
    print(window["kCGWindowNumber"] as! Int)
    break
}
SWIFT
)"
    [[ -n "$WID" ]] || { echo "no window named $VM_NAME — is its display open?" >&2; exit 1; }
    screencapture -x -o -l "$WID" "$OUT"
    echo "$OUT"
    exit 0
fi

# A script rather than a command line: quoting does not survive the trip
# through the guest agent intact.
LOCAL="/tmp/vm-shot.$$"
REMOTE="/tmp/vm-shot.$$.sh"
cat > "$LOCAL" <<EOF
#!/bin/bash
export DISPLAY=\${DISPLAY:-:0}
for candidate in /run/user/\$(id -u)/gdm/Xauthority "\$HOME/.Xauthority"; do
    [ -f "\$candidate" ] && export XAUTHORITY="\$candidate" && break
done
target="$TARGET"
size="$SIZE"
if [ -n "\$target" ]; then
    wid=\$(xdotool search --onlyvisible --name "\$target" | head -1)
    [ -n "\$wid" ] || { echo "no visible window matching: \$target" >&2; exit 1; }

    # Resize only when asked. A GTK window's reported frame includes its
    # shadow, so measuring that against the screen and "fitting" shrinks
    # windows that were never too big — which crops the header bar off
    # the top of the picture.
    # Moving a maximized window un-maximizes it, so only reposition when
    # a size was asked for and the window is being staged deliberately.
    if [ -n "\$size" ]; then
        xdotool windowsize "\$wid" \${size%x*} \${size#*x}
        xdotool windowmove "\$wid" 0 0
    fi
    xdotool windowactivate "\$wid" 2>/dev/null || true
    sleep 1.5
    import -window "\$wid" /tmp/vm-shot.png
    # A GTK window's drawable includes its shadow, which lands as a wide
    # black margin once the desktop behind it is dark. Trim it back to
    # the window itself.
    convert /tmp/vm-shot.png -bordercolor black -border 1 \\
        -fuzz 2% -trim +repage /tmp/vm-shot.png 2>/dev/null || true
else
    import -window root /tmp/vm-shot.png
fi
EOF

"$UTMCTL" file push "$VM_NAME" "$REMOTE" < "$LOCAL"
rm -f "$LOCAL"
"$UTMCTL" exec "$VM_NAME" --cmd /bin/sh -c \
    "chmod +x $REMOTE; chown $VM_USER $REMOTE; su - $VM_USER -c $REMOTE > $REMOTE.log 2>&1"
"$UTMCTL" file pull "$VM_NAME" /tmp/vm-shot.png > "$OUT" 2>/dev/null || true

if [[ ! -s "$OUT" ]]; then
    rm -f "$OUT"
    echo "capture failed; the guest said:" >&2
    "$UTMCTL" exec "$VM_NAME" --cmd /bin/cat "$REMOTE.log" >&2 || true
    exit 1
fi
echo "$OUT"
