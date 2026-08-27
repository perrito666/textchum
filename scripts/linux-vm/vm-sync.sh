#!/usr/bin/env bash
# Copy the working tree into the guest, then optionally run something.
#
#   ./vm-sync.sh                          just sync
#   ./vm-sync.sh 'cd textchum/linux && cargo build'
#
# rsync rather than git: the point is to build what is in front of you,
# including changes not committed yet.
set -euo pipefail

VM_NAME="${VM_NAME:-textchum-linux}"
VM_USER="${VM_USER:-textchum}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
KEY="$HERE/build/id_ed25519"
UTMCTL="/Applications/UTM.app/Contents/MacOS/utmctl"

IP="$("$UTMCTL" ip-address "$VM_NAME" 2>/dev/null | grep -E '^(192|10|172)\.' | head -1 || true)"
[[ -n "$IP" ]] || { echo "$VM_NAME is not reporting an address" >&2; exit 1; }

SSH=(ssh -i "$KEY" -o IdentitiesOnly=yes -o StrictHostKeyChecking=accept-new
     -o UserKnownHostsFile="$HERE/build/known_hosts")

rsync -az --delete \
    --exclude '.git' --exclude 'target' --exclude '.build' --exclude 'site' \
    --exclude 'scripts/linux-vm/build' \
    -e "${SSH[*]}" "$REPO/" "$VM_USER@$IP:~/textchum/"

[[ $# -eq 0 ]] && { echo "synced to $IP"; exit 0; }
exec "${SSH[@]}" "$VM_USER@$IP" "source ~/.cargo/env; $*"
