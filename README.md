# Network Daemon

Network Daemon is a daemon to manage Routing, Network Interface Control, DHCP and WiFi Connection on FreeBSD, like Network Manager on Linux. This project is under active development and not ready to use, and some of the documents is out of date.

## Current Status

The daemon can automatically manage existing interfaces, acquire DHCPv4
leases, and persist network profiles. On FreeBSD it also detects wireless
parent devices and creates station-mode WLAN interfaces when needed.

## TODO

- [x] WiFi Manager
    - [x] Auto launch wpa_supplicant
    - [x] Parse neccessary wpa_supplicant event
    - [x] Connect to WiFi via wpa_supplicant (Open and WPA2/WPA)
    - [x] Parse scan results obtained from wpa_supplicant
    - [x] Handle IPC request
- [x] Interface Manager
    - [x] Query Interface Information via Netlink
    - [x] Listen to Netlink `RTMGRP_LINK` group to monitor interface changes
    - [x] Handle IPC request
    - [x] Automatically detect and add interface for wireless device
- [x] Netlink
    - [x] Netlink Socket implementation
    - [x] A Stream-like Netlink Listener
        - [x] Netlink Listener to listen `RTMGRP_LINK`
        - [x] A Generic Netlink Listener to listen any group
- [ ] Route Manager
    - [ ] Auto select best route
    - [ ] Conectivity detect
    - [x] Auto remove dead route (Like two interface connect to same network, one disconnect will remain a dead one on the system, and unable to correctly set default route)
    - [ ] Migrate to `Netlink` protocol
- [ ] DHCP Manager
    - [x] DHCPv4
    - [ ] DHCPv6
- [ ] Storage Manager
    - [x] Configuration storage
    - [x] Encrypted password storage
- [ ] Network Daemon
    - [ ] Wireup all subsystem
    - [x] Handle IPC request
    - [x] Auto create WiFiManager actor
## TUI Client & the `network` group

The daemon's control socket (`/var/run/network-daemon/network-daemon.sock`) is
restricted to the `network` group (owned `network:network`, mode `0660`). To use
the TUI client as a normal user you must be a member of that group:

```sh
sudo pw groupmod network -m $USER
# then log out and back in (or reconnect SSH/session) for it to take effect
```

Build and run the TUI on the FreeBSD host:

```sh
cargo build -p network-manager-tui
sudo -u $USER ./target/debug/network-manager-tui
```

The TUI starts at the nmtui-style activity picker. Use the arrow keys and
Enter to choose an activity; Esc returns to the previous screen. In the
connection activation list, `r` rescans Wi-Fi. Arrow keys move within the
current list or form; Tab switches between the list/form and its action area.
On vertical action columns, Up/Down selects a button and Tab returns to the
list. The editor also supports Space for checkboxes and `<Cancel>`/`<OK>`
actions.

If you get `Permission denied`, it means your session doesn't yet have the
`network` group (re-login) — the daemon is working correctly.

To override the socket path: set `NETWORK_DAEMON_SOCK=/path/to/network-daemon.sock`.

## Wireless interface creation

During an interface refresh, the daemon reads FreeBSD's
`net.wlan.devices` list. For each wireless parent that has no WLAN child, it
creates the next available `wlanN` interface in station mode. Missing
configuration uses these defaults. To leave a device to another interface
manager, disable automatic creation for that parent:

```toml
[interface.iwn0]
create_wlan = false
```

Adding a `[interface.<device>.wlan]` table also opts that device out of the
default automatic path so a custom manager can apply those parameters.

The TUI's `Add` flow creates an interface, not just a network profile. For a
Wi-Fi interface it first asks the daemon for the detected parent devices, then
opens the editor; the `AddLink` request is sent only after the editor's `OK`.
Bridge, Bond/Team (`lagg`), and VLAN interfaces use the same delayed-create
flow. The daemon persists the link creation parameters under the interface's
`creation` entry, so a missing logical interface is recreated on a later
refresh or daemon restart. For example:

```toml
[interface.wlan0.creation]
type = "Wlan"
parent = "iwn0"

[interface.vlan10.creation]
type = "Vlan"
parent = "em0"
tag = 10
```

Logical-link creation, parameter setup, and destruction are performed through
FreeBSD kernel ioctl interfaces. The daemon does not invoke FreeBSD's
interface-management command-line utility or link its user-space library for
this path.

Interfaces supplied by the kernel (for example physical Ethernet and
kernel-created tunnel devices) are configured when detected; they are not
cloned by the `Add` flow. Saving a profile still persists non-sensitive
network metadata in `config.toml` and credentials in the encrypted SQLite
store.

## Building / Artifacts

Dev builds happen inside the lima `freebsd` VM (read-only host mount
requires `CARGO_TARGET_DIR=$HOME/nd-target`). Built binaries:
- Daemon: `$HOME/nd-target/debug/network-daemon`
- TUI: `$HOME/nd-target/debug/network-manager-tui`
