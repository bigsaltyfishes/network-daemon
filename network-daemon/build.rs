//! Build script for wutil-rs
//! Generates FFI bindings for libifconfig using bindgen

use std::{env, path::PathBuf};

fn main() {
    // Tell cargo to look for shared libraries in the specified directory
    println!("cargo:rustc-link-search=/usr/local/lib/");

    // Tell cargo to tell rustc to link shared libraries
    println!("cargo:rustc-link-lib=80211");

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
        // Only bind necessary constants
        .allowlist_var("AF_NETLINK")
        .allowlist_var("NETLINK_ROUTE")
        .allowlist_var("RTMGRP_LINK")
        .allowlist_var("RTMGRP_IPV4_IFADDR")
        .allowlist_var("RTMGRP_IPV6_IFADDR")
        .allowlist_var("NETLINK_GET_STRICT_CHK")
        .allowlist_var("SOL_NETLINK")
        .allowlist_var("SIOCIFCREATE2")
        .allowlist_var("SIOCGIFFLAGS")
        .allowlist_var("SIOCSIFFLAGS")
        .allowlist_var("SIOCGIFINFO_IN6")
        .allowlist_var("SIOCSIFINFO_IN6")
        .allowlist_var("IFF_UP")
        .allowlist_var("ND6_IFF_ACCEPT_RTADV")
        // lib80211 functions
        .allowlist_function("lib80211_alloc_regdata")
        .allowlist_function("lib80211_regdomain_findbyname")
        .allowlist_function("lib80211_regdomain_findbysku")
        .allowlist_function("lib80211_country_findbyname")
        .allowlist_function("lib80211_country_findbycc")
        // Wlan ioctl constants and types (might be picked up by headers but
        // explicit is safe)
        .allowlist_type("ieee80211_clone_params")
        .allowlist_var("IEEE80211_IOC_.*")
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
