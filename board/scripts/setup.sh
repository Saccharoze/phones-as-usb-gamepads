#!/usr/bin/env bash
# Builds the Sending Stones USB gadget (N HID gamepads) through configfs.
#
# Usage: sudo setup.sh up|down|status
#
# Environment:
#   DESCRIPTOR  HID report descriptor (from `stonesd --write-descriptor`)
#   CONFIG      config.toml; `pad_count = N` sets the pad count (default 4)
#   PADS        overrides the pad count
#   TAKEOVER=1  unbind another gadget that holds the UDC; `down` rebinds it
#   WITH_ECM=1  also expose a USB Ethernet (CDC ECM) function, so SSH over
#               USB keeps working during development. Off for play: Windows
#               has no inbox ECM driver.
#   ECM_ADDR    board address moved onto that link (default 10.55.0.1/24)
#   ECM_HOST_ADDR, ECM_DEV_ADDR
#               MAC addresses of that link, host side and board side. Unset,
#               the kernel draws random ones at each start; set them to keep
#               the host's interface name and network profile.
set -euo pipefail

GADGET_NAME=sending_stones
GADGET_ROOT=/sys/kernel/config/usb_gadget
G=$GADGET_ROOT/$GADGET_NAME
DESCRIPTOR=${DESCRIPTOR:-/usr/local/share/sending-stones/pad_descriptor.bin}
CONFIG=${CONFIG:-/etc/sending-stones/config.toml}
STATE_DIR=/run/sending-stones
DISPLACED_FILE=$STATE_DIR/displaced-gadget
REPORT_LENGTH=13
ECM_ADDR=${ECM_ADDR:-10.55.0.1/24}

die() { echo "setup.sh: $*" >&2; exit 1; }
log() { echo "setup.sh: $*"; }

require_root() {
    [[ $EUID -eq 0 ]] || die "must run as root"
}

pad_count() {
    if [[ -n ${PADS:-} ]]; then
        echo "$PADS"
    elif [[ -r $CONFIG ]] && grep -qE '^\s*pad_count\s*=' "$CONFIG"; then
        grep -E '^\s*pad_count\s*=' "$CONFIG" | tail -1 | tr -dc '0-9'
    else
        echo 4
    fi
}

first_udc() {
    ls /sys/class/udc 2>/dev/null | head -1
}

board_serial() {
    local dt=/sys/firmware/devicetree/base/serial-number
    if [[ -r $dt ]]; then
        tr -d '\0' <"$dt"
    elif grep -q '^Serial' /proc/cpuinfo; then
        awk '/^Serial/ {print $3}' /proc/cpuinfo
    else
        head -c 16 /etc/machine-id
    fi
}

# Prints the name of every other gadget currently bound to $1.
gadgets_holding() {
    local udc=$1 g
    for g in "$GADGET_ROOT"/*/; do
        g=${g%/}
        [[ $(basename "$g") == "$GADGET_NAME" ]] && continue
        [[ -r $g/UDC && $(cat "$g/UDC") == "$udc" ]] && basename "$g"
    done
    return 0
}

ensure_configfs() {
    modprobe libcomposite 2>/dev/null || true
    mountpoint -q /sys/kernel/config || mount -t configfs none /sys/kernel/config
    [[ -d $GADGET_ROOT ]] || die "no $GADGET_ROOT: kernel lacks USB gadget configfs (CONFIG_USB_CONFIGFS)"
}

free_udc() {
    local udc=$1 holders
    holders=$(gadgets_holding "$udc")
    [[ -z $holders ]] && return 0
    [[ ${TAKEOVER:-0} == 1 ]] || die "UDC $udc is held by gadget '$holders'; rerun with TAKEOVER=1 to replace it until 'down'"
    mkdir -p "$STATE_DIR"
    echo "$holders" >"$DISPLACED_FILE"
    echo "" >"$GADGET_ROOT/$holders/UDC"
    log "unbound gadget '$holders' (rebound on 'down')"
}

create_gadget() {
    local pads=$1 i f
    mkdir -p "$G"
    echo 0x1d6b >"$G/idVendor"
    echo 0x0104 >"$G/idProduct"
    echo 0x0100 >"$G/bcdDevice"
    echo 0x0200 >"$G/bcdUSB"
    mkdir -p "$G/strings/0x409"
    echo "Sending Stones" >"$G/strings/0x409/manufacturer"
    echo "Sending Stones Pad" >"$G/strings/0x409/product"
    board_serial >"$G/strings/0x409/serialnumber"

    mkdir -p "$G/configs/c.1/strings/0x409"
    echo "Sending Stones" >"$G/configs/c.1/strings/0x409/configuration"
    echo 500 >"$G/configs/c.1/MaxPower"

    for ((i = 1; i <= pads; i++)); do
        f=$G/functions/hid.pad$i
        mkdir -p "$f"
        echo 0 >"$f/protocol"
        echo 0 >"$f/subclass"
        echo $REPORT_LENGTH >"$f/report_length"
        # The pad has no output report, so it needs no OUT endpoint. The ZERO 3W's controller
        # has only five, and four pads plus the USB network link would take all of them.
        if [[ -e $f/no_out_endpoint ]]; then
            echo 1 >"$f/no_out_endpoint"
        fi
        cat "$DESCRIPTOR" >"$f/report_desc"
        ln -s "$f" "$G/configs/c.1/"
    done

    if [[ ${WITH_ECM:-0} == 1 ]]; then
        f=$G/functions/ecm.dev
        mkdir -p "$f"
        if [[ -n ${ECM_HOST_ADDR:-} ]]; then
            echo "$ECM_HOST_ADDR" >"$f/host_addr"
        fi
        if [[ -n ${ECM_DEV_ADDR:-} ]]; then
            echo "$ECM_DEV_ADDR" >"$f/dev_addr"
        fi
        ln -s "$f" "$G/configs/c.1/"
    fi
}

# The old link keeps the address after its gadget is unbound; move it so
# replies leave through the link that is actually connected.
claim_address() {
    local ifname=$1 other
    [[ -n $ifname ]] || return 0
    for other in $(ip -o addr show to "${ECM_ADDR%/*}" | awk '{print $2}'); do
        [[ $other != "$ifname" ]] && ip addr del "$ECM_ADDR" dev "$other"
    done
    ip addr replace "$ECM_ADDR" dev "$ifname"
    ip link set "$ifname" up
    log "board address $ECM_ADDR on $ifname"
}

ecm_ifname() {
    local f
    for f in "$GADGET_ROOT/$1"/functions/ecm.*; do
        [[ -r $f/ifname ]] && cat "$f/ifname" && return 0
    done
    return 0
}

wait_for_hidg() {
    local pads=$1 tries
    for ((tries = 0; tries < 20; tries++)); do
        (($(ls /dev/hidg* 2>/dev/null | wc -l) >= pads)) && return 0
        sleep 0.1
    done
    die "bound, but /dev/hidg* did not appear"
}

cmd_up() {
    require_root
    ensure_configfs
    local udc pads
    udc=$(first_udc)
    [[ -n $udc ]] || die "/sys/class/udc is empty: the OTG port is in host mode. See 'OTG port in host mode' in docs/01-install.md"
    if [[ -r $G/UDC && $(cat "$G/UDC") == "$udc" ]]; then
        log "already up on $udc"
        cmd_status
        return 0
    fi
    [[ -d $G ]] && teardown
    pads=$(pad_count)
    [[ $pads =~ ^[1-9][0-9]*$ ]] || die "bad pad count '$pads'"
    [[ -s $DESCRIPTOR ]] || die "descriptor $DESCRIPTOR missing; run: stonesd --write-descriptor $DESCRIPTOR"

    # From here a failure would leave the port with no gadget at all, and a
    # board reachable only over USB with no way back in: undo everything.
    trap rollback_up EXIT
    free_udc "$udc"
    create_gadget "$pads"
    echo "$udc" >"$G/UDC"
    wait_for_hidg "$pads"
    if [[ ${WITH_ECM:-0} == 1 ]]; then
        claim_address "$(ecm_ifname "$GADGET_NAME")"
    fi
    trap - EXIT
    log "up: $pads pad(s) on $udc"
    cmd_status
}

rollback_up() {
    local status=$?
    set +e
    log "up failed (exit $status): rolling back"
    teardown
    restore_displaced
    exit "$status"
}

# Removes our gadget; safe on a half-built tree.
teardown() {
    [[ -d $G ]] || return 0
    [[ -n $(cat "$G/UDC" 2>/dev/null) ]] && echo "" >"$G/UDC"
    find "$G/configs" -mindepth 2 -maxdepth 2 -type l -delete
    rmdir "$G"/configs/*/strings/* 2>/dev/null || true
    rmdir "$G"/configs/* 2>/dev/null || true
    rmdir "$G"/functions/* 2>/dev/null || true
    rmdir "$G"/strings/* 2>/dev/null || true
    rmdir "$G"
}

restore_displaced() {
    [[ -r $DISPLACED_FILE ]] || return 0
    local name udc
    name=$(cat "$DISPLACED_FILE")
    udc=$(first_udc)
    if [[ -d $GADGET_ROOT/$name && -n $udc ]]; then
        echo "$udc" >"$GADGET_ROOT/$name/UDC"
        log "rebound gadget '$name'"
        claim_address "$(ecm_ifname "$name")"
    fi
    rm -f "$DISPLACED_FILE"
}

cmd_down() {
    require_root
    if [[ -d $G ]]; then
        teardown
        log "down"
    else
        log "already down"
    fi
    restore_displaced
}

cmd_status() {
    local udc f devnum
    echo "UDCs:"
    for udc in /sys/class/udc/*; do
        [[ -e $udc ]] || { echo "  (none: OTG port in host mode)"; break; }
        echo "  $(basename "$udc") state=$(cat "$udc/state" 2>/dev/null || echo ?)"
    done
    echo "Gadgets:"
    for f in "$GADGET_ROOT"/*/; do
        [[ -d $f ]] || continue
        echo "  $(basename "$f") bound=$(cat "$f/UDC" 2>/dev/null || true)"
    done
    [[ -d $G ]] || { echo "$GADGET_NAME: down"; return 0; }
    echo "$GADGET_NAME functions:"
    for f in "$G"/functions/*; do
        devnum=$(cat "$f/dev" 2>/dev/null || true)
        if [[ -n $devnum ]]; then
            echo "  $(basename "$f") -> /dev/$(grep -oP 'DEVNAME=\K.*' "/sys/dev/char/$devnum/uevent")"
        else
            echo "  $(basename "$f")"
        fi
    done
}

case ${1:-} in
    up) cmd_up ;;
    down) cmd_down ;;
    status) cmd_status ;;
    *) die "usage: $0 up|down|status" ;;
esac
