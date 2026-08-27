# The Linux development VM

Textchum's Linux shell is a GTK4 and libadwaita app, and the things it
has to be good at only exist inside a real desktop session: the file
chooser and its portal, libadwaita following the system colour scheme,
the accelerators GNOME reserves for itself, client-side decorations,
the WebKit view the Markdown preview lives in, and how any of it looks.
A headless container can tell you the code runs. It cannot tell you the
app is right.

So: one virtual machine, holding two tiers.

* A **GNOME session**, logged in automatically, for integration, for
  judging how the app feels, and for documentation screenshots.
* A **headless compositor** (`weston --backend=headless`) in the same
  guest for the fast scripted loop — real libadwaita widgets rendered
  into a framebuffer, enough to catch a layout fault without waiting
  for a desktop.

The harness comes from its sibling project, prchum, where it was built;
this copy differs only in what gets installed and what gets driven.

## Building it

Needs UTM and `qemu-img` (`brew install qemu`) — UTM keeps its own copy
of qemu-img as a library it loads in-process, which the shell cannot
call.

```sh
./create-vm.sh
```

Downloads the Ubuntu 24.04 LTS arm64 cloud image, grows a disk from it,
writes a cloud-init seed carrying a key generated for this machine, and
creates and starts the VM through UTM's scripting interface. It prints
the guest's address and the commands that provision it.

The first call also needs macOS to let this terminal drive UTM. macOS
asks once, as a dialog nobody sees if the run is unattended; until it
is answered, every scripting call fails with **AppleEvent timed out
(-1712)**. Grant it under System Settings → Privacy & Security →
Automation.

The VM is disposable by design. When it drifts, throw it away and build
another:

```sh
./create-vm.sh --delete
```

Stop it first — `--delete` will not remove a running machine, and it
removes `build/` either way, so a half-delete leaves a machine UTM
still lists and a build directory that no longer describes it.

Never kill the QEMU process to end a stuck machine: the guest's disk is
mid-write and comes back corrupted, which shows up as a boot that
freezes a second in or a desktop session that never paints. Ask UTM to
stop it, and if UTM's own state sticks at *stopping*, quit and reopen
UTM rather than reaching for the process.

Everything downloaded or generated lands in `build/`, which is ignored.
Nothing identifying is committed: `user-data.template` carries a
placeholder, and the key is generated at build time.

## Working with it

```sh
./vm-ssh.sh                        a shell in the guest
./vm-ssh.sh 'cargo build'          one command
./vm-sync.sh 'cd textchum/linux && cargo build'   push this tree, then build
./vm-shot.sh shot.png              a picture of its screen
./vm-shot.sh shot.png "Textchum"   just that window
./vm-exec.sh 'systemctl status'    when the network is not up yet
```

`vm-ssh.sh` looks the address up every time, and uses a key generated
for this machine alone. Both of those are deliberate.

The address changes: the guest takes a fresh DHCP lease when it
reboots, and a remembered address then fails as **no route to host** —
which reads like a broken network rather than a stale number.

The key is the machine's own, without a passphrase, kept in `build/`.
A personal key usually has a passphrase and needs an agent that scripts
do not have, and a disposable VM has no business holding one.

`vm-exec.sh` goes over the guest agent's virtio-serial channel instead
of the network, which is what you want before the guest has an address,
or when it has stopped answering and you need to find out why.

`vm-sync.sh` rsyncs the working tree rather than cloning it, so what
gets built is what is in front of you, uncommitted changes included.

## Screenshots, and why the session runs on Xorg

Under Wayland the session cannot be photographed or driven. GNOME's
`Shell.Screenshot` answers *"Screenshot is not allowed"* to anything
that is not an interactive user action, and `grim` does nothing under
Mutter, which implements none of the wlroots screencopy protocol.

So `provision.sh` sets `WaylandEnable=false` and the session runs on
Xorg, where `import -window <id>` captures one window cleanly and
`xdotool` can type and click. `vm-shot.sh` takes a window title, and
only resizes when asked, because moving a maximized window
un-maximizes it and "fitting" one that was never too big crops its
header bar away.

`--host` falls back to capturing UTM's own window from the outside, for
when the session has deliberately been switched to Wayland to check
fractional scaling or anything else Wayland-specific.

## Driving the app

The harness mirrors the macOS one. There, screens are opened through
hidden `--debug-panel` hooks and photographed with `screencapture`;
here, the same hooks exist in the GTK shell, and GTK publishes its
widget tree over **AT-SPI**, so a script can find a button by its
label, click it, and read a list's row count.

That correspondence is the point: the same question — *is what the
accessibility tree claims actually painted?* — can be asked on both
platforms, and a screenshot answers it either way.

## The documentation screenshots

`shot.sh` and `capture-tour.sh` run inside the guest and produce the
Linux half of the tour — every screen, in light and dark, framed the
same way each time. Copy them over, give the guest the demo project,
and run them:

```sh
scp shot.sh capture-tour.sh guest:/tmp/
./vm-ssh.sh '/tmp/capture-tour.sh light'
./vm-ssh.sh '/tmp/capture-tour.sh dark'
```

Two things that are not obvious. Anything a language server answers —
hover, completion, the outline — needs the demo project to be a real
crate with a `Cargo.toml`, or rust-analyzer indexes nothing and the
panels come up empty. And the pointer is parked off the window before
every capture: left where the last run put it, it rests over whatever
appears underneath and the hover popover walks into the picture.

Panels are their own X windows, so they are captured by window rather
than cropped out of the screen — which also means a popover never
carries the desktop behind it.

Point a run at a scratch configuration, never the real one:

```sh
./vm-ssh.sh 'textchum/linux/target/debug/textchum-gtk \
    --fresh --config /tmp/scratch/config.json /tmp/demo/file.rs'
```
