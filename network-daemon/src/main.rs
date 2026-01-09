use std::path::PathBuf;

use kameo::{actor::Spawn, mailbox};
use tracing::{error, info};

use crate::{daemon::NetworkDaemon, wifi::WpaSupplicant};

mod daemon;
mod dhcp;
mod ffi;
mod interface;
mod netlink;
mod route;
mod wifi;

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    tracing_subscriber::fmt::init();

    std::panic::set_hook(Box::new(|info| {
        error!("Panic occurred: {:?}", info);
    }));

    info!("Starting network-daemon...");

    let working_dir = PathBuf::from("/var/run/network-daemon");
    let daemon = NetworkDaemon::<WpaSupplicant>::spawn_with_mailbox(
        working_dir,
        mailbox::unbounded(),
    );
    daemon.wait_for_shutdown().await;
}
