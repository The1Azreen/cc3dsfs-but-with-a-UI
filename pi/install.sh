#!/bin/bash
# Installs cc3dsfs as a kiosk on Raspberry Pi OS Lite (64-bit): the Pi boots
# straight into cc3dsfs on HDMI, with no desktop.
#
# Tested target: Raspberry Pi 400 / Pi 4. Other 64-bit Pis with the default
# vc4-kms-v3d display driver (Pi 3, Pi 5, CM4) should work the same way.
#
# Usage (from a clone of the repo, or an extracted release archive):
#   sudo ./pi/install.sh                 build (if needed) and install
#   sudo ./pi/install.sh --force-hdmi    also force HDMI on at boot (for
#                                        upscalers that are switched on after the Pi)
#   sudo ./pi/install.sh --uninstall     remove the kiosk, restore the console
set -euo pipefail

PI_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(dirname "$PI_DIR")"
APP_USER=cc3dsfs
APP_HOME=/var/lib/cc3dsfs
FORCE_HDMI=0
UNINSTALL=0

for arg in "$@"; do
    case "$arg" in
        --force-hdmi) FORCE_HDMI=1 ;;
        --uninstall) UNINSTALL=1 ;;
        *) echo "Unknown option: $arg" >&2; exit 1 ;;
    esac
done

if [ "$(id -u)" -ne 0 ]; then
    echo "Run this with sudo: sudo $0 $*" >&2
    exit 1
fi

log() { echo "==> $*"; }

boot_dir() {
    if [ -e /boot/firmware/config.txt ]; then echo /boot/firmware; else echo /boot; fi
}

if [ "$UNINSTALL" -eq 1 ]; then
    log "Removing the cc3dsfs kiosk"
    systemctl disable --now cc3dsfs.service 2>/dev/null || true
    rm -f /etc/systemd/system/cc3dsfs.service /usr/local/bin/cc3dsfs-kiosk \
        /etc/udev/rules.d/99-cc3dsfs.rules /etc/sudoers.d/cc3dsfs
    rm -rf /opt/cc3dsfs
    systemctl daemon-reload
    udevadm control --reload-rules
    systemctl enable getty@tty1.service
    log "Done. /etc/cc3dsfs and the $APP_USER user (with its settings in $APP_HOME) were kept."
    log "Reboot to get the normal console back: sudo reboot"
    exit 0
fi

if [ "$(uname -m)" != "aarch64" ]; then
    echo "This needs 64-bit Raspberry Pi OS (aarch64); found $(uname -m)." >&2
    exit 1
fi

# Install the first package name that exists (names differ between Debian releases).
apt_install_any() {
    for pkg in "$@"; do
        if apt-cache show "$pkg" >/dev/null 2>&1; then
            apt-get install -y --no-install-recommends "$pkg"
            return 0
        fi
    done
    echo "None of these packages are available: $*" >&2
    return 1
}

log "Installing runtime packages"
apt-get update
apt-get install -y --no-install-recommends \
    sudo libinput10 libxkbcommon0 libgbm1 libdrm2 libegl1 libgles2 \
    libudev1 fontconfig fonts-dejavu-core
apt_install_any libasound2t64 libasound2

# Use a prebuilt binary next to this script (release archive), otherwise build.
BINARY=""
if [ -x "$PI_DIR/cc3dsfs" ]; then
    BINARY="$PI_DIR/cc3dsfs"
    log "Using the prebuilt binary $BINARY"
else
    log "No prebuilt binary found, building from source (this takes a while on a Pi)"
    apt-get install -y --no-install-recommends \
        build-essential pkg-config curl ca-certificates \
        libasound2-dev libudev-dev libinput-dev libxkbcommon-dev \
        libgbm-dev libdrm-dev libfontconfig1-dev
    BUILD_USER="${SUDO_USER:-root}"
    BUILD_HOME="$(getent passwd "$BUILD_USER" | cut -d: -f6)"
    if ! sudo -u "$BUILD_USER" -H bash -lc 'command -v cargo' >/dev/null 2>&1 \
        && [ ! -x "$BUILD_HOME/.cargo/bin/cargo" ]; then
        log "Installing Rust for $BUILD_USER"
        sudo -u "$BUILD_USER" -H bash -c \
            'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal'
    fi
    # Full LTO with one codegen unit needs lots of RAM and time on a Pi.
    sudo -u "$BUILD_USER" -H bash -c "cd '$REPO_DIR' && \
        CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=8 \
        \"\$HOME/.cargo/bin/cargo\" build --release"
    BINARY="$REPO_DIR/target/release/cc3dsfs"
fi

log "Installing cc3dsfs"
install -d /opt/cc3dsfs /etc/cc3dsfs
install -m 0755 "$BINARY" /opt/cc3dsfs/cc3dsfs
install -m 0755 "$PI_DIR/cc3dsfs-kiosk" /usr/local/bin/cc3dsfs-kiosk
if [ ! -e /etc/cc3dsfs/kiosk.conf ]; then
    install -m 0644 "$PI_DIR/kiosk.conf" /etc/cc3dsfs/kiosk.conf
else
    log "Keeping the existing /etc/cc3dsfs/kiosk.conf"
fi

log "Creating the $APP_USER user"
if ! id "$APP_USER" >/dev/null 2>&1; then
    useradd --system --home-dir "$APP_HOME" --create-home --shell /usr/sbin/nologin "$APP_USER"
fi
for group in video render input audio plugdev; do
    getent group "$group" >/dev/null && usermod -aG "$group" "$APP_USER"
done

log "Allowing the kiosk to shut down and restart the Pi"
cat > /etc/sudoers.d/cc3dsfs <<EOF
$APP_USER ALL=(root) NOPASSWD: /usr/bin/systemctl poweroff, /usr/bin/systemctl reboot
EOF
chmod 0440 /etc/sudoers.d/cc3dsfs
visudo -cf /etc/sudoers.d/cc3dsfs >/dev/null

log "Installing USB permissions for the capture boards"
install -m 0644 "$PI_DIR/99-cc3dsfs.rules" /etc/udev/rules.d/99-cc3dsfs.rules
udevadm control --reload-rules
udevadm trigger --subsystem-match=usb

BOOT="$(boot_dir)"
log "Checking $BOOT/config.txt"
if ! grep -q '^dtoverlay=vc4-kms-v3d' "$BOOT/config.txt"; then
    log "Enabling the KMS display driver (dtoverlay=vc4-kms-v3d)"
    printf '\n[all]\ndtoverlay=vc4-kms-v3d\n' >> "$BOOT/config.txt"
fi

CMDLINE="$BOOT/cmdline.txt"
add_cmdline() {
    # cmdline.txt is a single line; append the option if it is missing.
    local key="${1%%=*}"
    if ! grep -q "\(^\| \)$key=" "$CMDLINE"; then
        sed -i "1 s|\$| $1|" "$CMDLINE"
    fi
}
# Hide the blinking console cursor behind the kiosk and keep the console awake.
add_cmdline "vt.global_cursor_default=0"
add_cmdline "consoleblank=0"
if [ "$FORCE_HDMI" -eq 1 ]; then
    MODE_LINE="$(. /etc/cc3dsfs/kiosk.conf; echo "${MODE:-1920x1080@60}")"
    OUTPUT_NAME="$(. /etc/cc3dsfs/kiosk.conf; echo "${OUTPUT:-HDMI-A-1}")"
    log "Forcing $OUTPUT_NAME on at $MODE_LINE even when no display is detected"
    add_cmdline "video=$OUTPUT_NAME:${MODE_LINE}D"
fi

log "Enabling the kiosk service"
install -m 0644 "$PI_DIR/cc3dsfs.service" /etc/systemd/system/cc3dsfs.service
systemctl daemon-reload
systemctl disable getty@tty1.service
systemctl enable cc3dsfs.service

cat <<EOF

cc3dsfs kiosk installed.
  Reboot to start it:        sudo reboot
  Display settings:          /etc/cc3dsfs/kiosk.conf
  Logs:                      journalctl -u cc3dsfs -f
  Stop it (over SSH):        sudo systemctl stop cc3dsfs
  Remove the kiosk:          sudo $0 --uninstall
EOF
