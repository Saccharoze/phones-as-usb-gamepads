#!/usr/bin/env bash
# Joins the board to a Wi-Fi network through netplan (the board has no
# NetworkManager). Prompts interactively so the password never appears in
# shell history or on the command line.
#
# Usage, from the dev machine: ssh -t root@<board> /usr/local/lib/sending-stones/join-wifi.sh
set -euo pipefail

NETPLAN_FILE=/etc/netplan/20-sending-stones-wifi.yaml
IFACE=${IFACE:-wlan0}

[[ $EUID -eq 0 ]] || { echo "must run as root" >&2; exit 1; }

# YAML double-quoted string: escape backslash and double quote.
yaml_quote() {
    local s=${1//\\/\\\\}
    printf '"%s"' "${s//\"/\\\"}"
}

read -rp "Wi-Fi network name (SSID): " ssid
read -rsp "Wi-Fi password: " password
echo
[[ -n $ssid ]] || { echo "empty SSID" >&2; exit 1; }

umask 077
cat >"$NETPLAN_FILE" <<EOF
network:
  version: 2
  renderer: networkd
  wifis:
    $IFACE:
      dhcp4: true
      access-points:
        $(yaml_quote "$ssid"):
          password: $(yaml_quote "$password")
EOF
chmod 600 "$NETPLAN_FILE"

netplan apply
echo "waiting for an address on $IFACE..."
for _ in $(seq 1 30); do
    addr=$(ip -4 -br addr show "$IFACE" | awk '{print $3}')
    if [[ -n $addr ]]; then
        echo "connected: $IFACE $addr"
        exit 0
    fi
    sleep 1
done
echo "no address after 30 s; check: journalctl -u systemd-networkd -u netplan-wpa-$IFACE --no-pager | tail" >&2
exit 1
