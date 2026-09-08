//! Generate bindings for FreeBSD's kernel network-control ABI.

use std::{env, path::PathBuf};

fn main() {
    // Rerun if wrapper.h changes
    println!("cargo:rerun-if-changed=wrapper.h");

    // Generate bindings
    let bindings = bindgen::Builder::default()
        .header("wrapper.h")
        .clang_arg("-I/usr/local/include/")
        // Only bind necessary types
        .allowlist_type("sockaddr_nl")
        .allowlist_type("in6_ndireq")
        .allowlist_type("nlmsghdr")
        .allowlist_type("ifdrv")
        .allowlist_type("ifbreq")
        .allowlist_type("vlanreq")
        .allowlist_type("lagg_reqport")
        .allowlist_type("lagg_reqall")
        // Only bind necessary constants
        .allowlist_var("AF_NETLINK")
        .allowlist_var("NETLINK_ROUTE")
        .allowlist_var("RTMGRP_LINK")
        .allowlist_var("RTMGRP_IPV4_IFADDR")
        .allowlist_var("RTMGRP_IPV6_IFADDR")
        .allowlist_var("NETLINK_GET_STRICT_CHK")
        .allowlist_var("SOL_NETLINK")
        .allowlist_var("ND_.*")
        .allowlist_var("IFF_UP")
        .allowlist_var("ND6_IFF_ACCEPT_RTADV")
        // Wlan clone parameters are passed directly to SIOCIFCREATE2.
        .allowlist_type("ieee80211_clone_params")
        .allowlist_type("ieee80211_opmode")
        .prepend_enum_name(false)
        .clang_macro_fallback()
        // Generate for no_std compatibility
        .use_core()
        .generate()
        .expect("Unable to generate bindings");

    // Write bindings to OUT_DIR
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("freebsd.rs"))
        .expect("Couldn't write bindings!");
}
