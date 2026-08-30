use std::{path::Path, pin::Pin, time::Duration};

use futures_lite::{Stream, stream};
use libnetwork_daemon::error::WifiError;
use tokio::process::Command;

/// Handle for a wpa_supplicant instance
pub struct WpaSupplicantInstanceHandle {
    child: Pin<Box<dyn Stream<Item = std::process::ExitStatus> + Send + Sync>>,
}

impl WpaSupplicantInstanceHandle {
    pub async fn new<P>(workdir: P, iface: &str) -> Result<Self, WifiError>
    where
        P: AsRef<Path> + Send,
    {
        if tokio::fs::metadata(workdir.as_ref().join(iface))
            .await
            .is_ok()
        {
            // Socket already exists, assume wpa_supplicant is running
            return Err(WifiError::WpaSupplicantRunning(format!(
                "wpa_supplicant already running or socket exists at {:?}",
                workdir.as_ref().join(iface)
            )));
        }

        let child = Command::new("wpa_supplicant")
            .arg("-s")
            .arg("-i")
            .arg(iface)
            .arg("-D")
            .arg("bsd")
            .arg("-i")
            .arg(iface)
            .arg("-C")
            .arg(workdir.as_ref())
            .spawn()
            .map_err(|e| WifiError::SupplicantStartFailed(e.to_string()))?;

        // Wait for socket to be created
        let socket_path = workdir.as_ref().join(iface);
        let start = std::time::Instant::now();
        while tokio::fs::metadata(&socket_path).await.is_err() {
            if start.elapsed() > Duration::from_millis(5000) {
                return Err(WifiError::SupplicantStartFailed(
                    "Operation timeout".to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        let stream =
            Box::pin(stream::unfold(Some(child), |child_opt| async move {
                if let Some(mut child) = child_opt {
                    match child.wait().await {
                        Ok(status) => Some((status, None)),
                        Err(_) => None,
                    }
                } else {
                    None
                }
            }));

        Ok(WpaSupplicantInstanceHandle { child: stream })
    }
}

impl Stream for WpaSupplicantInstanceHandle {
    type Item = std::process::ExitStatus;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.child.as_mut().poll_next(cx)
    }
}
