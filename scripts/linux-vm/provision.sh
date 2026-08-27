#!/usr/bin/env bash
# Runs inside the guest, over SSH, after cloud-init has finished.
#
# Two tiers live in this one machine: a real GNOME session for
# integration, feel, and documentation screenshots, and a headless
# compositor for the fast scripted loop. Both need the same packages,
# which is the reason for one VM rather than two.
#
# Re-running is safe.
set -euo pipefail

echo "==> GNOME desktop"
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
    ubuntu-desktop-minimal \
    gnome-console

# ---------------------------------------------------------------------
# Project packages. Swap this block for whatever the thing you are
# testing needs to build — Qt, Electron, SDL, a language runtime. The
# rest of this script is the part that makes the machine testable at
# all, and is worth keeping whatever you build in it.
# ---------------------------------------------------------------------
echo "==> GTK4 / libadwaita development packages"
# The versions Ubuntu 24.04 ships (GTK 4.14, libadwaita 1.5) are exactly
# what the shell asks for, so nothing needs pinning older. libsoup comes
# with WebKit on some setups and not others; naming it is cheaper than
# diagnosing its absence.
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
    libgtk-4-dev \
    libadwaita-1-dev \
    libgtksourceview-5-dev \
    libwebkitgtk-6.0-dev \
    libsoup-3.0-dev \
    libssl-dev

echo "==> What the editor's optional features shell out to"
# Prose spell check runs hunspell; Jump to Definition falls back to a
# Universal Ctags index when no language server answers; the scripted
# language server used by the smoke test is a Python script.
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
    hunspell \
    hunspell-en-us \
    universal-ctags \
    python3

echo "==> Testing harness"
# at-spi2-core is the accessibility bus: the analogue of the macOS
# Accessibility API that lets the harness find a button by name, click
# it, and count rows in a list. grim and weston cover the headless tier;
# gnome-screenshot covers the desktop one.
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y \
    at-spi2-core \
    libatspi2.0-dev \
    python3-pyatspi \
    weston \
    xdotool \
    imagemagick \
    x11-apps \
    gnome-screenshot \
    xdg-desktop-portal-gnome

echo "==> Rust toolchain"
if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --no-modify-path
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
rustc --version
# --no-modify-path above keeps rustup out of the profile, which then
# makes this machine unlike the one a user has: ~/.cargo/bin is missing
# from the login shell's PATH, and anything that asks a login shell
# where a tool is — the Flatpak build does, to find language servers on
# the host — correctly reports that it is nowhere.
grep -q '.cargo/env' "$HOME/.profile" 2>/dev/null \
    || printf '\n. "$HOME/.cargo/env"\n' >> "$HOME/.profile"

echo "==> An Xorg session"
# Deliberate, and the single most useful thing here. Under Wayland the
# session cannot be photographed or driven: GNOME's Shell.Screenshot
# refuses any caller that is not an interactive user, and Mutter
# implements none of the wlroots screencopy protocol grim needs. On Xorg
# `import -window <id>` captures one window cleanly and xdotool can type
# and click.
#
# The cost is that this is not Wayland, so Wayland-specific behaviour —
# fractional scaling in particular — is not exercised. Comment the line
# out and reboot to check the app under Wayland, then put it back.
WAYLAND_LINE="WaylandEnable=false"

echo "==> Automatic login"
# Screenshots need a session running without someone typing a password
# into the console first.
sudo install -d /etc/gdm3
sudo tee /etc/gdm3/custom.conf >/dev/null <<CONF
[daemon]
AutomaticLoginEnable=true
AutomaticLogin=textchum
$WAYLAND_LINE
CONF

echo "==> Accessibility bus on for the session"
# GTK only exposes its widget tree over AT-SPI when this is set, and the
# harness is useless without it.
sudo tee /etc/environment.d/90-a11y.conf >/dev/null <<'CONF'
GTK_A11Y=atspi
CONF
gsettings set org.gnome.desktop.interface toolkit-accessibility true || true

echo "==> No screen lock, ever"
# A development machine that locks itself is worse than it sounds. The
# lock screen covers the session, but the windows underneath keep their
# saved pixmaps — so `import -window` still returns a picture, of the
# application as it looked before the lock. Screenshots quietly stop
# tracking reality, and every one of them looks plausible.
gsettings set org.gnome.desktop.screensaver lock-enabled false || true
gsettings set org.gnome.desktop.screensaver idle-activation-enabled false || true
gsettings set org.gnome.desktop.session idle-delay 0 || true
sudo systemctl mask sleep.target suspend.target hibernate.target >/dev/null || true

echo "==> Silencing the first-run wizard"
# Ubuntu's welcome tour sits on top of everything, which is no way to
# photograph an application.
sudo mkdir -p /home/textchum/.config
echo "yes" | sudo tee /home/textchum/.config/gnome-initial-setup-done >/dev/null
sudo chown -R textchum:textchum /home/textchum/.config

echo "==> Graphical target"
sudo systemctl set-default graphical.target

echo
echo "Provisioned. Reboot for the GNOME session to come up logged in:"
echo "    sudo reboot"
