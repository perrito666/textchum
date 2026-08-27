#!/bin/bash
# Every screen in the documentation tour, from the GTK shell, in one
# appearance. Runs INSIDE the guest beside shot.sh; the pictures land
# in /tmp/shots and are pulled back with scp.
#
#   ./vm-ssh.sh '/tmp/capture-tour.sh light'
#   ./vm-ssh.sh '/tmp/capture-tour.sh dark'
#
# It wants the Harbor demo project at /tmp/harbor with a Cargo.toml, so
# rust-analyzer has something to answer about.
#
#   capture-tour.sh light|dark
set -u
A="$1"
S=/tmp/shot.sh
H=/tmp/harbor
[ "$A" = "dark" ] && SUF="-dark" || SUF=""
# rust-analyzer has to index before anything it answers is worth a
# picture; everything LSP-shaped waits this long first.
LSP=25

SIDEBAR=1 $S "editor$SUF"    "$A" 6  main  "$H/src/harbor.rs"
SIDEBAR=1 $S "diagnostics$SUF" "$A" 6 main "$H/src/harbor.rs" sleep:40
$S "preview$SUF"             "$A" 6  main  "$H/README.md"
$S "spell-check$SUF"         "$A" 7  main  "$H/docs/notes.md"
# Hover is its own surface, so it is captured as a panel: rest the
# pointer on Harbor in `pub struct Harbor` and wait for the server.
$S "hover$SUF"               "$A" 5  panel "$H/src/harbor.rs" sleep:$LSP click:279,223 sleep:5
$S "completion$SUF"          "$A" 5  panel "$H/src/harbor.rs" sleep:$LSP ctrl+Home rep:22:Down rep:12:Right type:. sleep:4
$S "outline$SUF"             "$A" 4  panel "$H/src/harbor.rs" sleep:$LSP ctrl+shift+o sleep:3
$S "palette$SUF"             "$A" 3  panel "$H/src/harbor.rs" ctrl+shift+p sleep:2
$S "open-quickly$SUF"        "$A" 4  panel "$H/src/harbor.rs" ctrl+p sleep:2 type:client sleep:2
$S "find-in-project$SUF"     "$A" 5  panel "$H/src/harbor.rs" ctrl+shift+f sleep:2 type:berth sleep:3
$S "new-with-format$SUF"     "$A" 3  panel "$H/README.md" ctrl+shift+n sleep:2
$S "server-status$SUF"       "$A" 4  panel "$H/src/main.rs" sleep:$LSP ctrl+shift+p sleep:2 type:Language_Server sleep:2 Return sleep:3
PREFS=1 $S "settings-general$SUF" "$A" 4 panel "$H/Makefile" sleep:2
GLOBS=1 $S "hide-globs$SUF"  "$A" 4  main  "$H/Makefile" sleep:2
echo "=== $A done: $(ls /tmp/shots/*.png | wc -l) files ==="
