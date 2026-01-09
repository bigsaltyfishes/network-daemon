use std::{net::Ipv4Addr, pin::Pin, time::Duration};

use futures_lite::{Stream, stream};
use kameo::{
    Actor,
    actor::ActorRef,
    error::SendError,
    message::StreamMessage,
    prelude::{Context, Message},
};
use tokio::time::Instant;
use tokio_util::task::AbortOnDropHandle;
use tracing::{info, warn};

use crate::dhcp::client::{DhcpClient, DhcpClientError};

#[derive(Debug)]
pub enum LeaseEvent {
    Renewal,
    Rebinding,
    LeaseExpired,
}

pub struct LeaseWatchdog {
    renewal_time_val: u32,
    rebinding_time_val: u32,
    lease_time_val: u32,
    client_ref: ActorRef<DhcpClient>,
    timer_handle: Option<
        AbortOnDropHandle<
            Result<
                Pin<Box<dyn Stream<Item = usize> + Send>>,
                SendError<StreamMessage<usize, (), ()>>,
            >,
        >,
    >,
}

impl LeaseWatchdog {
    pub fn new(
        renewal_time: u32,
        rebinding_time: u32,
        lease_time: u32,
        client_ref: ActorRef<DhcpClient>,
    ) -> Self {
        Self {
            renewal_time_val: renewal_time,
            rebinding_time_val: rebinding_time,
            lease_time_val: lease_time,
            client_ref,
            timer_handle: None,
        }
    }

    pub async fn set_timers(&mut self, actor_ref: &ActorRef<Self>) {
        let start = tokio::time::Instant::now();
        let renewal_deadline =
            start + Duration::from_secs(self.renewal_time_val as u64);
        let rebinding_deadline =
            start + Duration::from_secs(self.rebinding_time_val as u64);
        let lease_deadline =
            start + Duration::from_secs(self.lease_time_val as u64);

        let stream = stream::unfold(0, move |state| async move {
            let stages = [renewal_deadline, rebinding_deadline, lease_deadline];

            if state >= stages.len() {
                return None;
            }

            tokio::time::sleep_until(stages[state]).await;
            let now = Instant::now();

            for (idx, &deadline) in stages.iter().enumerate().rev() {
                if now >= deadline {
                    return Some((idx, idx + 1));
                }
            }

            None
        });

        self.timer_handle = Some(AbortOnDropHandle::new(
            actor_ref.attach_stream(Box::pin(stream), (), ()),
        ));
    }
}

impl Actor for LeaseWatchdog {
    type Args = Self;
    type Error = ();

    async fn on_start(
        mut args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        args.set_timers(&actor_ref).await;

        Ok(args)
    }
}

impl Message<StreamMessage<usize, (), ()>> for LeaseWatchdog {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<usize, (), ()>,
        ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        match msg {
            StreamMessage::Next(event) => {
                let instant = Instant::now();
                info!("LeaseWatchdog: Timer event {} at {:?}", event, instant);
                let new_times = match event {
                    0 => {
                        info!("Lease Renewal time reached");
                        self.client_ref.ask(LeaseEvent::Renewal).await
                    }
                    1 => {
                        info!("Lease Rebinding time reached");
                        self.client_ref.ask(LeaseEvent::Rebinding).await
                    }
                    2 => {
                        info!("Lease Expired");
                        self.client_ref.ask(LeaseEvent::LeaseExpired).await
                    }
                    _ => return,
                };

                match new_times {
                    Ok((renewal, rebinding, lease)) => {
                        self.renewal_time_val = renewal;
                        self.rebinding_time_val = rebinding;
                        self.lease_time_val = lease;
                        self.set_timers(ctx.actor_ref()).await;
                    }
                    Err(e) => match e.unwrap_err() {
                        DhcpClientError::ServerRejected => {
                            warn!(
                                "LeaseWatchdog: Lease operation rejected by \
                                 server"
                            );
                            ctx.actor_ref().kill();
                        }
                        e => {
                            info!(
                                "LeaseWatchdog: Lease operation failed: {:?}, \
                                 waiting for next timeout.",
                                e
                            );
                        }
                    },
                };
            }
            _ => {}
        }
    }
}
