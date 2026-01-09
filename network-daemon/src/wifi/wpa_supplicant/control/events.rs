use std::{
    io::{Read, Write},
    path::Path,
    task::Poll,
};

use async_io::Async;
use futures_lite::{AsyncReadExt, AsyncWriteExt, Stream};
use futures_util::FutureExt;
use libnetwork_daemon::{
    WpaCommand, WpaEvent,
    error::{WpaCtrlError, WpaSocketError},
};

use super::{WPA_MAX_REPLY_SIZE, socket::WpaSocket};

pub struct WpaEventListener(Async<WpaSocket>);

impl WpaEventListener {
    pub async fn open<P, R>(
        bind_path: P,
        ctrl_path: R,
    ) -> Result<Self, WpaCtrlError>
    where
        P: AsRef<Path> + Send,
        R: AsRef<Path> + Send,
    {
        let ctrl_path = ctrl_path.as_ref();
        let local_path = bind_path.as_ref();

        // Remove existing socket file if present
        // TODO: Handle this in upper layers.
        let _ = tokio::fs::remove_file(&local_path).await;

        // Create and bind local socket
        let socket = WpaSocket::connect(&local_path, ctrl_path).await?;

        // Attach to receive events
        let mut ret = Self(
            Async::new(socket).map_err(|e| WpaCtrlError::IoError(e.into()))?,
        );
        ret.attach().await?;

        Ok(ret)
    }

    pub async fn attach(&mut self) -> Result<(), WpaCtrlError> {
        // Send ATTACH command
        self.0.write_all(b"ATTACH").await.map_err(|e| {
            WpaCtrlError::SendFailed {
                cmd: WpaCommand::Attach,
                source: WpaSocketError::SendFailed(e.into()),
            }
        })?;

        Ok(())
    }

    pub async fn detach(&mut self) -> Result<(), WpaCtrlError> {
        // Send DETACH command
        self.0.write_all(b"DETACH").await.map_err(|e| {
            WpaCtrlError::SendFailed {
                cmd: WpaCommand::Detach,
                source: WpaSocketError::SendFailed(e.into()),
            }
        })?;

        Ok(())
    }

    pub async fn wait_event(&mut self) -> Result<WpaEvent, WpaCtrlError> {
        let mut buf = vec![0u8; WPA_MAX_REPLY_SIZE];

        let len = self
            .0
            .read(&mut buf)
            .await
            .map_err(|e| WpaSocketError::RecvFailed(e.into()))?;

        let event = String::from_utf8_lossy(&buf[..len]).trim_end().to_string();
        Ok(WpaEvent::parse(&event))
    }
}

impl Stream for WpaEventListener {
    type Item = Result<WpaEvent, WpaCtrlError>;

    fn poll_next(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let mut buf = vec![0u8; WPA_MAX_REPLY_SIZE];

        match futures_lite::ready!(this.0.read(&mut buf).poll_unpin(cx)) {
            Ok(len) => {
                if len == 0 {
                    return Poll::Ready(None);
                }
                let event =
                    String::from_utf8_lossy(&buf[..len]).trim_end().to_string();
                Poll::Ready(Some(Ok(WpaEvent::parse(&event))))
            }
            Err(e) => Poll::Ready(Some(Err(WpaSocketError::RecvFailed(
                e.into(),
            )
            .into()))),
        }
    }
}
