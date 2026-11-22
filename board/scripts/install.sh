#!/usr/bin/env bash
# Installs Sending Stones on the board from a staging directory holding the
# stonesd binary and every file of board/scripts.
# Idempotent. Keeps an existing /etc/sending-stones/config.toml and gadget.env.
#
# Usage: sudo ./install.sh [--dev]
#   --dev  set TAKEOVER=1 WITH_ECM=1 in gadget.env (SSH over USB survives),
#          keeping its other lines (the link's MAC addresses live there)
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ETC=/etc/sending-stones
LIB=/usr/local/lib/sending-stones
SHARE=/usr/local/share/sending-stones
USER_NAME=sending-stones
DEV=0
[[ ${1:-} == --dev ]] && DEV=1

[[ $EUID -eq 0 ]] || { echo "install.sh: must run as root" >&2; exit 1; }

set_gadget_env() {
    local key=$1 value=$2 file=$ETC/gadget.env
    touch "$file"
    if grep -q "^$key=" "$file"; then
        sed -i "s/^$key=.*/$key=$value/" "$file"
    else
        echo "$key=$value" >>"$file"
    fi
}

if ! id "$USER_NAME" >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin "$USER_NAME"
fi

install -d "$ETC" "$LIB" "$SHARE"
install -m 755 "$HERE/stonesd" /usr/local/bin/
install -m 755 "$HERE/setup.sh" "$HERE/join-wifi.sh" "$LIB/"
/usr/local/bin/stonesd --write-descriptor "$SHARE/pad_descriptor.bin"

[[ -e $ETC/config.toml ]] || install -m 644 "$HERE/config.example.toml" "$ETC/config.toml"
if [[ ! -e $ETC/gadget.env || $DEV == 1 ]]; then
    set_gadget_env TAKEOVER 1
    set_gadget_env WITH_ECM "$DEV"
fi

install -m 644 "$HERE/99-sending-stones.rules" /etc/udev/rules.d/
udevadm control --reload
# Pads that already exist were created before the rule; apply it to them now.
udevadm trigger --subsystem-match=hidg --action=add
udevadm settle
install -m 644 "$HERE/sending-stones-gadget.service" "$HERE/sending-stones.service" /etc/systemd/system/
systemctl daemon-reload
systemctl enable sending-stones-gadget.service sending-stones.service >/dev/null 2>&1

echo "install.sh: installed; gadget.env: $(tr '\n' ' ' <"$ETC/gadget.env")"
