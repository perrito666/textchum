#!/bin/bash
# Captures one Textchum screen from the GTK shell. Runs INSIDE the
# guest — copy it over and call it through vm-ssh.sh:
#
#   scp shot.sh capture-tour.sh guest:/tmp/
#   ./vm-ssh.sh '/tmp/capture-tour.sh light'
#
#   shot.sh <name> <light|dark> <wait> <main|panel> [file] [step...]
#
# Steps are applied in order, a second apart: a bare word is a key for
# xdotool, `type:some_text` types it (underscores become spaces),
# `rep:N:key` repeats a key, `click:X,Y` clicks a screen position,
# `tab:N` clicks the Nth tab of the frontmost dialog, `panelmove:X,Y`
# parks that dialog, and `sleep:N` waits.
set -u
NAME="$1"; APPEARANCE="$2"; WAIT="$3"; TARGET="$4"; FILE="${5:-}"; shift 5 || true
export DISPLAY=:0
export XDG_CONFIG_HOME=/tmp/shots-profile
export PATH="$HOME/.cargo/bin:$PATH"
# Ubuntu 24.04 denies WebKit's bubblewrap sandbox the user namespace
# it wants, and a screenshot run is not the place to explain that.
# The CI job sets the same variable for the same reason.
export WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=1
mkdir -p /tmp/shots-profile/textchum /tmp/shots
cat > /tmp/shots-profile/textchum/config.json <<CONF
{
  "appearance": "$APPEARANCE",
  "editor": { "font_size": 13, "spell": "en_US" },
  "workspace": {
    "ctags_fallback": false,
    "hide": [".*", "target", "Cargo.lock"]
  }
}
CONF
pkill -f "textchum-g[t]k"; sleep 1
[ -n "${GLOBS:-}" ] && export TEXTCHUM_DEBUG_GLOBS=1
[ -n "${PREFS:-}" ] && export TEXTCHUM_DEBUG_PREFS=1
[ -n "${MENU:-}" ] && export TEXTCHUM_DEBUG_MENU=1
[ -n "${SIDEBAR:-}" ] && export TEXTCHUM_DEBUG_SIDEBAR=1
setsid ~/textchum/linux/target/debug/textchum-gtk --fresh ${FILE:+$FILE} \
    > /tmp/shot-app.log 2>&1 < /dev/null &
sleep 6
# Size the main window so every shot frames the same way.
MAIN=$(for i in $(xdotool search --class textchum 2>/dev/null); do
    xwininfo -id $i | grep -q IsViewable && echo $i; done | head -1)
[ -n "$MAIN" ] && { xdotool windowmove $MAIN 40 40; xdotool windowsize $MAIN 1180 740; }
sleep 2
[ -n "$MAIN" ] && xdotool windowactivate $MAIN
sleep 1
# PARK THE POINTER outside the window before anything else. Left
# where the last run put it, it rests over whatever appears under
# it and the mouse-rest hover popover gatecrashes the screenshot.
xdotool mousemove 1700 950
sleep 1
for KEY in "$@"; do
    case "$KEY" in
        sleep:*) sleep "${KEY#sleep:}" ;;
        type:*)  xdotool type --delay 40 "$(printf %s "${KEY#type:}" | tr _ " ")" ;;
        # Park a dialog at a known spot so the clicks that follow can
        # be written down. Where GTK first puts it depends on the
        # window manager and the screen, so nothing else can.
        panelmove:*) P="${KEY#panelmove:}"
            D=$(for i in $(xdotool search --class textchum 2>/dev/null); do
                xwininfo -id $i | grep -q IsViewable || continue
                G=$(xdotool getwindowgeometry $i | awk "/Geometry/ {print \$2}")
                echo "$(( ${G%x*} * ${G#*x} )) $i"
              done | sort -n | head -1 | cut -d" " -f2)
            [ -n "$D" ] && xdotool windowmove $D ${P//,/ } ;;
        # Click one of a preferences window's three tabs, found from
        # the window's own geometry: where the window manager puts
        # the dialog is not ours to predict, so nothing is hardcoded
        # but the tab centres as fractions of its width.
        tab:*)  N="${KEY#tab:}"
            D=$(for i in $(xdotool search --class textchum 2>/dev/null); do
                xwininfo -id $i | grep -q IsViewable || continue
                G=$(xdotool getwindowgeometry $i | awk "/Geometry/ {print \$2}")
                echo "$(( ${G%x*} * ${G#*x} )) $i"
              done | sort -n | head -1 | cut -d" " -f2)
            [ -z "$D" ] && continue
            eval "$(xdotool getwindowgeometry --shell $D)"
            case "$N" in
                1) FX=32 ;;
                2) FX=50 ;;
                *) FX=68 ;;
            esac
            xdotool mousemove $(( X + WIDTH * FX / 100 )) $(( Y + 78 )) click 1 ;;
        rep:*)   R="${KEY#rep:}"; xdotool key --repeat "${R%%:*}" --repeat-delay 30 --clearmodifiers "${R#*:}" ;;
        click:*) C="${KEY#click:}"; xdotool mousemove ${C//,/ } click 1 ;;
        *)       xdotool key --clearmodifiers "$KEY" ;;
    esac
    sleep 1
done
sleep "$WAIT"
if [ "$TARGET" = "panel" ]; then
    # The smallest viewable app window: the dialog or popover on top.
    WID=$(for i in $(xdotool search --class textchum 2>/dev/null); do
        xwininfo -id $i | grep -q IsViewable || continue
        G=$(xdotool getwindowgeometry $i | awk "/Geometry/ {print \$2}")
        W=${G%x*}; H=${G#*x}
        echo "$((W*H)) $i"
    done | sort -n | head -1 | cut -d" " -f2)
else
    WID=$MAIN
fi
[ -n "$WID" ] && import -window "$WID" "/tmp/shots/$NAME.png"
echo "$NAME -> $(identify -format "%wx%h" /tmp/shots/$NAME.png 2>/dev/null)"
