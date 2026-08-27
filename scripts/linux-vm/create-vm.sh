#!/usr/bin/env bash
# Builds the Linux VM textchum's GTK shell is developed and photographed
# in. The machine is meant to be disposable: delete it and run this
# again rather than repairing it by hand.
#
#   ./create-vm.sh            build it
#   ./create-vm.sh --delete   throw it away
#
# QEMU backend rather than Apple Virtualization: it boots a stock cloud
# image from UEFI without a separate kernel and initrd, and on Apple
# silicon an aarch64 guest still runs on the hardware hypervisor, so the
# predictability costs nothing.
set -euo pipefail

VM_NAME="${VM_NAME:-textchum-linux}"
MEMORY_MIB="${MEMORY_MIB:-8192}"
CPU_CORES="${CPU_CORES:-4}"
DISK_SIZE="${DISK_SIZE:-64G}"
# The machine gets its own key, generated on first build. A personal
# key usually carries a passphrase, which needs an agent no script has,
# and a disposable VM has no business holding one anyway.
VM_KEY_NAME="id_ed25519"

# Ubuntu 24.04 LTS: GNOME 46, GTK 4.14, libadwaita 1.5, and the same
# distribution the Linux CI job would run on.
IMAGE_URL="https://cloud-images.ubuntu.com/releases/24.04/release/ubuntu-24.04-server-cloudimg-arm64.img"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD="$HERE/build"
# UTM ships qemu-img as a library it loads in-process, not as something
# that can be run, so the host needs its own copy. It is wanted for one
# thing only: the cloud image's disk is about 3.5 GiB, which a desktop
# does not fit into.
QEMU_IMG="$(command -v qemu-img || true)"
UTMCTL="/Applications/UTM.app/Contents/MacOS/utmctl"

die() { echo "error: $*" >&2; exit 1; }

vm_exists() { "$UTMCTL" list | awk 'NR>1 {$1=""; $2=""; sub(/^ +/,""); print}' | grep -qx "$VM_NAME"; }

if [[ "${1:-}" == "--delete" ]]; then
    vm_exists && "$UTMCTL" stop "$VM_NAME" 2>/dev/null || true
    vm_exists && "$UTMCTL" delete "$VM_NAME"
    rm -rf "$BUILD"
    echo "deleted $VM_NAME"
    exit 0
fi

[[ -x "$UTMCTL" ]] || die "UTM not found in /Applications"
[[ -n "$QEMU_IMG" ]] || die "qemu-img not found — brew install qemu"
vm_exists && die "$VM_NAME already exists — ./create-vm.sh --delete first"

mkdir -p "$BUILD"

echo "==> Base image"
BASE="$BUILD/$(basename "$IMAGE_URL")"
if [[ ! -f "$BASE" ]]; then
    curl -fL --progress-bar -o "$BASE.part" "$IMAGE_URL"
    mv "$BASE.part" "$BASE"
fi

echo "==> Disk"
DISK="$BUILD/$VM_NAME.qcow2"
if [[ ! -f "$DISK" ]]; then
    cp "$BASE" "$DISK"
    "$QEMU_IMG" resize "$DISK" "$DISK_SIZE"
fi

echo "==> Machine key"
KEY="$BUILD/$VM_KEY_NAME"
[[ -f "$KEY" ]] || ssh-keygen -t ed25519 -N "" -C "$VM_NAME vm" -f "$KEY" >/dev/null

echo "==> cloud-init seed"
# The key is read at build time rather than committed: the template
# stays free of anything identifying.
SEED_SRC="$BUILD/seed"
rm -rf "$SEED_SRC" && mkdir -p "$SEED_SRC"
sed "s|__SSH_PUBLIC_KEY__|$(cat "$KEY.pub")|" \
    "$HERE/user-data.template" > "$SEED_SRC/user-data"
cat > "$SEED_SRC/meta-data" <<EOF
instance-id: $VM_NAME
local-hostname: $VM_NAME
EOF
SEED="$BUILD/seed.iso"
rm -f "$SEED"
# cloud-init finds the drive by its CIDATA volume label, and it scans
# every block device for it — so the seed rides in as an ordinary VirtIO
# disk. Attaching it as removable media instead makes UTM import an
# empty drive, and the guest then boots with no user and no key.
hdiutil makehybrid -iso -joliet -default-volume-name CIDATA \
    -o "${SEED%.iso}" "$SEED_SRC" >/dev/null

echo "==> Creating the virtual machine"
# The display is not optional: created without one, the machine boots
# fine and GNOME never starts a session, because there is no graphics
# device for it to open.
#
# UTM copies both disks into the VM bundle before the event returns, and
# the cloud image is large enough that this outruns AppleScript's default
# two-minute reply window: the script gets -1712 while UTM carries on
# importing. Ask for a longer one.
osascript <<APPLESCRIPT
with timeout of 1800 seconds
    tell application "UTM"
        set vm to make new virtual machine with properties {backend:qemu, configuration:{name:"$VM_NAME", architecture:"aarch64", memory:$MEMORY_MIB, cpu cores:$CPU_CORES, hypervisor:true, uefi:true, displays:{{hardware:"virtio-gpu-pci", dynamic resolution:true, native resolution:false}}, drives:{{removable:false, interface:VirtIO, source:POSIX file "$DISK"}, {removable:false, interface:VirtIO, source:POSIX file "$SEED"}}}}
        start vm
    end tell
end timeout
APPLESCRIPT

echo "==> Waiting for the guest to come up"
IP=""
for _ in $(seq 1 60); do
    IP="$("$UTMCTL" ip-address "$VM_NAME" 2>/dev/null | grep -E '^(192|10|172)\.' | head -1 || true)"
    [[ -n "$IP" ]] && break
    sleep 10
done
[[ -n "$IP" ]] || die "the guest never reported an address; open UTM and look at the console"

echo "==> Provisioning through the guest agent"
# Not over SSH: the agent's channel is virtio-serial and always
# reachable, while host-to-guest networking depends on a macOS privacy
# permission the script cannot grant. See README.md.
"$UTMCTL" file push "$VM_NAME" /root/provision.sh < "$HERE/provision.sh"
"$UTMCTL" exec "$VM_NAME" --cmd /bin/sh -c \
    'cp /root/provision.sh /home/textchum/ \
     && chown textchum:textchum /home/textchum/provision.sh \
     && chmod +x /home/textchum/provision.sh \
     && setsid su - textchum -c "/home/textchum/provision.sh > /home/textchum/provision.log 2>&1" \
        < /dev/null > /dev/null 2>&1 &'

cat <<EOF

$VM_NAME is up at $IP, and provisioning has started in the background.

Watch it:

    ./vm-exec.sh 'tail -f provision.log'

It installs GNOME, the GTK4 and libadwaita development packages, the
AT-SPI testing harness and Rust, which takes a while. Reboot when it
finishes and the machine comes up logged into GNOME:

    ./vm-exec.sh 'sudo reboot'

Then:

    ./vm-ssh.sh              a shell in the guest
    ./vm-shot.sh shot.png    a picture of its screen

Use those rather than a remembered address: the guest takes a new DHCP
lease when it reboots, and yesterday's address fails as "no route to
host", which reads like a broken network rather than a stale number.

EOF
