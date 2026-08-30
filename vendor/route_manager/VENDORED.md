# Vendored: route_manager v0.2.13

Vendored from [tun-rs/route_manager](https://github.com/tun-rs/route_manager)
(crates.io `route_manager` v0.2.13, Apache-2.0) because upstream fails to
compile on FreeBSD aarch64.

## Why / what we patched

`unix_bsd/mod.rs` (the `From<Ipv4Addr> for sockaddr_in` impl) hardcoded
`sin_zero: [0i8; 8]`. On FreeBSD aarch64 `c_char` is `u8` (it is `i8` on amd64),
so this won't compile on arm64 FreeBSD. The fix changes the initializer to a
target-typed `[0; 8]`, which matches `c_char` on every platform.

Fix location: `vendor/route_manager/src/unix_bsd/mod.rs`.

## Upstream PR

Submitted to tun-rs/route_manager (see task notes). Once a fixed release is
published, we should drop this vendored copy and go back to the crates.io
dependency.
