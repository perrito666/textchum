#!/usr/bin/env bash
# SSH into the guest, or run a command in it.
#
#   ./vm-ssh.sh                 a shell
#   ./vm-ssh.sh 'cargo build'   one command
#
# The address is looked up every time rather than remembered: the guest
# takes a fresh DHCP lease when it reboots, and a remembered address
# fails as "no route to host", which reads like a broken network instead
# of a stale number.
#
# The key is the machine's own, generated without a passphrase when the
# VM was built. Your personal key is not involved: it usually carries a
# passphrase, which needs an agent that a script does not have, and a
# disposable VM has no business holding it.
set -euo pipefail

VM_NAME="${VM_NAME:-textchum-linux}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KEY="$HERE/build/id_ed25519"
UTMCTL="/Applications/UTM.app/Contents/MacOS/utmctl"

[[ -f "$KEY" ]] || { echo "no key at $KEY — build the machine first" >&2; exit 1; }

IP="$("$UTMCTL" ip-address "$VM_NAME" 2>/dev/null | grep -E '^(192|10|172)\.' | head -1 || true)"
[[ -n "$IP" ]] || { echo "$VM_NAME is not reporting an address — is it started?" >&2; exit 1; }

exec ssh -i "$KEY" \
    -o IdentitiesOnly=yes \
    -o StrictHostKeyChecking=accept-new \
    -o UserKnownHostsFile="$HERE/build/known_hosts" \
    "textchum@$IP" "$@"
