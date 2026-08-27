#!/usr/bin/env bash
# Run a command in the guest and get its output back.
#
#   ./vm-exec.sh 'cargo --version'
#
# This goes over the guest agent's virtio-serial channel rather than the
# network, so it works whether or not the host can reach the guest — see
# the note about the Local Network permission in README.md.
#
# utmctl's own exec returns nothing for anything but the shortest
# commands, so the output is parked in a file and read back separately.
set -euo pipefail

VM_NAME="${VM_NAME:-textchum-linux}"
UTMCTL="/Applications/UTM.app/Contents/MacOS/utmctl"
USER_NAME="${VM_USER:-textchum}"

[[ $# -ge 1 ]] || { echo "usage: $(basename "$0") <command>" >&2; exit 2; }

COMMAND="$*"
OUT="/tmp/vm-exec.$$"

"$UTMCTL" exec "$VM_NAME" --cmd /bin/sh -c \
    "su - $USER_NAME -c $(printf '%q' "$COMMAND") > $OUT 2>&1; echo \$? >> $OUT"
"$UTMCTL" exec "$VM_NAME" --cmd /bin/cat "$OUT"
"$UTMCTL" exec "$VM_NAME" --cmd /bin/rm -f "$OUT"
