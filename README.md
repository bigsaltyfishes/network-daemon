# Network Daemon

Network Daemon is a daemon to manage Routing, Network Interface Control, DHCP and WiFi Connection on FreeBSD, like Network Manager on Linux. This project is under active development and not ready to use, and some of the documents is out of date.

## Current Status

Should work out of box, but we haven't implement local storge system yet, any change apply to daemon won't save (like known networks). Currently `Network Daemon` can automatically manage your exists interfaces, and automatically aquire DHCPv4 lease. Do not enable SLAAC for your interface or `Network Daemon` will assume your interface is managed externally and won't start internal DHCPv4 client for it, I will fix it later.

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
    - [ ] Automatically detect and add interface for wireless device
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
    - [ ] Configuration storage
    - [ ] Encrypted password storage
- [ ] Network Daemon
    - [ ] Wireup all subsystem
    - [x] Handle IPC request
    - [x] Auto create WiFiManager actor