use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};

use async_net::unix::{UnixListener, UnixStream};
use futures_lite::{Stream, stream};

pub struct StreamdListener {
    inner: Pin<Box<dyn Stream<Item = io::Result<UnixStream>> + Send>>,
}

impl From<UnixListener> for StreamdListener {
    fn from(listener: UnixListener) -> Self {
        let s = stream::unfold(listener, |listener| async move {
            match listener.accept().await {
                Ok((stream, _addr)) => Some((Ok(stream), listener)),
                Err(e) => Some((Err(e), listener)),
            }
        });
        Self { inner: Box::pin(s) }
    }
}

impl Stream for StreamdListener {
    type Item = io::Result<UnixStream>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}
