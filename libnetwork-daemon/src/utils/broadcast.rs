use async_channel::{Receiver, Sender};
use crossbeam_queue::SegQueue;

/// Broadcast utility for sending messages to multiple subscribers
///
/// # Type Parameters
///
/// - `T` - Type of messages to broadcast
///
/// # Examples
/// ```rust
/// use libnetwork_daemon::utils::Broadcast;
/// use futures_lite::future::block_on;
/// use async_channel::Receiver;
///
/// block_on(async {
///     let broadcaster = Broadcast::new();
///     let mut sub1: Receiver<String> = broadcaster.subscribe();
///     let mut sub2: Receiver<String> = broadcaster.subscribe();
///
///     broadcaster.broadcast("Hello, Subscribers!".to_string()).await;
///
///     assert_eq!(sub1.recv().await.unwrap(), "Hello, Subscribers!");
///     assert_eq!(sub2.recv().await.unwrap(), "Hello, Subscribers!");
/// });
/// ```
pub struct Broadcast<T>
where
    T: Send,
{
    senders: SegQueue<Sender<T>>,
}

impl<T> Broadcast<T>
where
    T: Send + Clone,
{
    pub fn new() -> Self {
        Self {
            senders: SegQueue::new(),
        }
    }

    /// Subscribe to broadcast messages
    ///
    /// # Returns
    ///
    /// Receiver for broadcast messages
    pub fn subscribe(&self) -> Receiver<T> {
        let (sender, receiver) = async_channel::unbounded();
        self.senders.push(sender);
        receiver
    }

    /// Broadcast a message to all subscribers
    ///
    /// # Arguments
    ///
    /// - `message` - Message to broadcast
    ///
    /// # Returns
    ///
    /// Number of successful deliveries
    pub async fn broadcast(&self, message: T) -> usize {
        let mut num = self.senders.len();
        let mut cnt = 0;

        while num > 0 {
            if let Some(sender) = self.senders.pop() {
                match sender.send(message.clone()).await {
                    Ok(_) => {
                        self.senders.push(sender);
                        cnt += 1;
                    }
                    Err(_) => {} // Receiver dropped, do not re-add
                }
            }
            num -= 1;
        }

        cnt
    }
}
