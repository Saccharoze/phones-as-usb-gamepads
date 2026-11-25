# Install

Everything is built on a Linux computer, from a clone of this repo. The board software goes over SSH, the app
goes on the phones as an APK, and nothing is installed on the PC you play on. During development the build
computer can also be that PC.

```mermaid
flowchart LR
  B["Build computer"] -- "SSH: stonesd, scripts" --> Z["Radxa ZERO 3W"]
  B -- "APK" --> P["Phones"]
  P -- "Wi-Fi" --> Z
  Z -- "USB" --> PC["PC you play on"]
```

## What you need

| For | What |
|---|---|
| Board | a Radxa ZERO 3W running Armbian, reachable over SSH as root. Use a key login: a deploy runs `ssh` and `scp` several times. Tested on Armbian 26.8.1 trixie with kernel 6.1.115-vendor-rk35xx. The kernel needs USB gadget configfs with HID (`CONFIG_USB_CONFIGFS`, `CONFIG_USB_CONFIGFS_F_HID`) |
| Board software | Rust stable, `zig` (from your distribution or ziglang.org), the `aarch64-unknown-linux-musl` target and `cargo-zigbuild` (commands below) |
| The app | the Android SDK with its licenses accepted (`sdkmanager --licenses`; the build fetches the API 37 platform it needs) and a full JDK 21. A Java runtime without `javac` fails with `does not provide the required capabilities: [JAVA_COMPILER]` |
| Phones | Android 8 or later |

```sh
rustup target add aarch64-unknown-linux-musl
cargo install cargo-zigbuild
```

## Check the OTG port

The ZERO 3W has two USB-C ports. The OTG port is the one on the right, with the pin header at the top (see
the photo in the [README](../README.md)). It also powers the board, and it has to run in device mode. On the
board:

```sh
ls /sys/class/udc
```

The tested board prints `fcc00000.usb`.

### OTG port in host mode

An empty list means the port runs in host mode, and `setup.sh` stops with an error that points here. To see
the mode:

```sh
tr -d '\0' < /proc/device-tree/usbdrd/usb@fcc00000/dr_mode
```

The tested image prints `otg`, with no overlay, and that is enough. For `host`, Armbian ships an overlay that
sets the port to peripheral mode: `/boot/dtb/rockchip/overlay/rk3568-dwc3-peripheral.dtbo`, which also covers the
board's RK3566. Add it to `/boot/armbianEnv.txt` (on the existing `overlays=` line if there is one), then
reboot. The boot script tries the name with its `overlay_prefix` first, then the plain name, which matches this
file:

```
overlays=rk3568-dwc3-peripheral
```

I haven't needed it, so this step is untested.

## Board software

The first deploy needs a network you can SSH into. A board with none yet: put its SD card in a Linux computer
and write this file as `etc/netplan/20-sending-stones-wifi.yaml` on its root partition, owned by root, mode
600. It is the file `join-wifi.sh` writes later.

```yaml
network:
  version: 2
  renderer: networkd
  wifis:
    wlan0:
      dhcp4: true
      access-points:
        "<network name>":
          password: "<password>"
```

Then, from the repo root:

```sh
STONES_SSH=root@<board-address> cargo xtask deploy
```

This builds `stonesd` as a static aarch64 binary, and copies it with every file of `board/scripts/` to
`/tmp/sending-stones-staging` on the board. Then it runs `install.sh` from there, starts the USB gadget and
restarts the daemon. Rerun it after changing the code or the scripts. It keeps an existing `config.toml` and
`gadget.env`.

| Installed | What it is |
|---|---|
| `/usr/local/bin/stonesd` | the daemon, run by systemd as the `sending-stones` user |
| `/usr/local/lib/sending-stones/` | `setup.sh` (builds the USB gadget), `join-wifi.sh` |
| `/usr/local/share/sending-stones/pad_descriptor.bin` | the HID descriptor, written by `install.sh` with `stonesd --write-descriptor` |
| `/etc/sending-stones/config.toml` | settings, all listed in `board/scripts/config.example.toml` |
| `/etc/sending-stones/gadget.env` | gadget options: `TAKEOVER`, `WITH_ECM` |
| `/etc/udev/rules.d/99-sending-stones.rules` | lets that user write `/dev/hidg*` |
| `/etc/systemd/system/sending-stones-gadget.service`, `sending-stones.service` | start both at boot |

`TAKEOVER=1` is on by default. If the board image binds its own USB gadget at boot, `setup.sh` unbinds it, and
gives the port back on `setup.sh down`.

A PIN makes a phone type 4 digits before it takes a player slot, free or held by another phone. Use one when
other people share the Wi-Fi: add `pin = 1234` (your own digits, no leading zero: 0042 is `pin = 42`) at the
top of `config.toml`, then `systemctl restart sending-stones`.

The board shows the PC four pads. For fewer, set `pad_count` in `config.toml` (for example `pad_count = 2`),
then `systemctl restart sending-stones-gadget sending-stones`. Restarting the gadget drops SSH over USB for a few
seconds.

Logs: `cargo xtask logs`, with the same `STONES_SSH`.

## SSH over USB, for development

```sh
cargo xtask deploy --dev
```

With the board plugged into the build computer, this sets `WITH_ECM=1`: the board also shows up as a USB
network adapter, with the board at `10.55.0.1`. The first time, the link doesn't exist yet: run it over Wi-Fi
with `STONES_SSH` set, then `systemctl restart sending-stones-gadget` on the board, or reboot it. Give the
computer's side an address in `10.55.0.0/24`, on the interface that appears when the board is plugged in
(`ip -br link`), for example `sudo ip addr add 10.55.0.2/24 dev <interface>`, or with a NetworkManager profile
that keeps it across replugs. From then on, `cargo xtask` uses `root@10.55.0.1` when `STONES_SSH` is unset.
Optional: `ECM_HOST_ADDR` and `ECM_DEV_ADDR` in `gadget.env` fix the link's MAC addresses, so the PC keeps the
same interface name.

Turn it off for play, and always on Windows, which has no built-in driver for this kind of USB network adapter.
Over Wi-Fi SSH, since the USB link drops:

```sh
sed -i 's/^WITH_ECM=.*/WITH_ECM=0/' /etc/sending-stones/gadget.env
systemctl restart sending-stones-gadget
```

A deploy only starts the gadget, it never restarts it, so a changed `gadget.env` applies after that restart or
a reboot. A deploy without `--dev` leaves an existing `gadget.env` as it is.

## Wi-Fi

The board and the phones must share a network. Guest networks usually block traffic between devices. A phone
hotspot works if the board joins it.

```sh
ssh -t root@<board-address> /usr/local/lib/sending-stones/join-wifi.sh
```

It asks for the network name and password, writes `/etc/netplan/20-sending-stones-wifi.yaml` (readable by root
only), applies it and prints the address the board gets. Running it again replaces that network. It is written for netplan
with systemd-networkd, as on the tested image. Moving the board to another network cuts a Wi-Fi SSH session, so
do that over the USB link.

## The app

The prebuilt APK is on the [Releases page](https://github.com/Saccharoze/phones-as-usb-gamepads/releases/latest).
To build it yourself:

```sh
cd phone
echo "sdk.dir=$HOME/Android/Sdk" > local.properties    # or set ANDROID_HOME
JAVA_HOME=<path-to-jdk-21> ./gradlew assembleDebug
```

Then put the APK on each phone, either way (the path is the one built above, from `phone/`):

- `adb install -r app/build/outputs/apk/debug/app-debug.apk`
- copy the file to the phone and open it; Android asks to allow installs from the app that opened it.

The prebuilt APK and your own build are signed with different keys, so Android refuses to install one over the
other: uninstall the app first. A release build needs your own signing key: `phone/keystore.properties` with `storeFile`, `storePassword`,
`keyAlias` and `keyPassword` (git ignores it), then `./gradlew assembleRelease`.

## First test

1. Plug the board's OTG port, the USB-C port on the right, into the PC. The board boots and starts both
   services by itself.
2. Start Steam on the PC and leave it running. On Linux it keeps the pads open, and a Linux PC only polls a
   pad while something reads it.
3. Open Sending Stones on a phone. The board shows up as "Sending Stones on <board hostname>"; tap it. If it
   doesn't, type its address under "Or type its address".
4. Tap Player 1. The controller opens full screen, in landscape.
5. In Steam, open Settings, then Controller. All four pads are listed as "Sending Stones Pad". Open "Test Device
   Inputs" for one of them and press buttons on the phone: each one shows up under its own name.
6. Hold a stick and lock the phone. The pad returns to neutral.

Windows is not tested yet. Turn SSH over USB off before plugging into Windows.

## If something is off

| Sign | Check |
|---|---|
| The app doesn't list the board | Same Wi-Fi, not a guest network? `journalctl -u sending-stones -n 20` on the board should show `advertising over mDNS`. If it does, the network blocks mDNS: type the address |
| The controller shows "no reply" | Is the board still on the Wi-Fi? Its address may have changed (DHCP): go back and pick it again |
| "Every slot is held" | A dropped phone frees its slot after 2 minutes. The same phone gets its own slot back at once, marked "yours" |
| Steam doesn't list the pads | Is the cable in the OTG port? On Linux, `cat /proc/bus/input/devices` should list four "Sending Stones Pad" devices |
| `setup.sh`: `/sys/class/udc is empty` | [OTG port in host mode](#otg-port-in-host-mode) |
| Anything else | `cargo xtask logs` |

## Uninstall

On the board, over Wi-Fi SSH (stopping the gadget drops the USB link):

```sh
systemctl disable --now sending-stones.service sending-stones-gadget.service
rm -r /usr/local/bin/stonesd /usr/local/lib/sending-stones /usr/local/share/sending-stones /etc/sending-stones \
  /etc/udev/rules.d/99-sending-stones.rules /etc/systemd/system/sending-stones.service \
  /etc/systemd/system/sending-stones-gadget.service
systemctl daemon-reload
userdel sending-stones
```

Stopping the gadget gives the USB port back to the gadget it displaced, if any. The Wi-Fi network saved by
`join-wifi.sh` stays, in `/etc/netplan/20-sending-stones-wifi.yaml`.

[All steps](../README.md#steps) · Next: [How it works](02-how-it-works.md)
