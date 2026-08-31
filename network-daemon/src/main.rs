use std::path::PathBuf;

use kameo::{actor::Spawn, mailbox};
use tracing::{error, info};

use crate::{daemon::NetworkDaemon, wifi::WpaSupplicant};

mod config;
mod daemon;
mod dhcp;
mod ffi;
mod interface;
mod netlink;
mod route;
mod storage;
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

    // Reload the TOML config on SIGHUP (config reload is non-destructive: on a
    // parse error the previous in-memory guidance is kept).
    let config_path =
        std::path::PathBuf::from("/var/db/network-daemon/config.toml");
    tokio::spawn(async move {
        let mut sig = match tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::hangup(),
        ) {
            Ok(s) => s,
            Err(e) => {
                error!("failed to set up SIGHUP handler: {}", e);
                return;
            }
        };
        while sig.recv().await.is_some() {
            match crate::config::DaemonConfig::load(&config_path) {
                Ok(cfg) => {
                    info!(
                        "SIGHUP: reloaded config.toml ({} networks, {} interfaces)",
                        cfg.networks.len(),
                        cfg.interface.len()
                    );
                }
                Err(e) => {
                    error!(
                        "SIGHUP: config reload failed, keeping previous: {}",
                        e
                    );
                }
            }
        }
    });

    daemon.wait_for_shutdown().await;
}
