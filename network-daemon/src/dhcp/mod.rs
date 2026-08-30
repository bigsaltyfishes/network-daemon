mod client;
mod lease;
mod socket;
mod utils;

use std::{ops::ControlFlow, time::Duration};

use kameo::{
    Actor,
    actor::{ActorId, ActorRef, Spawn, WeakActorRef},
    error::ActorStopReason,
    mailbox,
    prelude::{Context, Message},
};
use libnetwork_daemon::{InterfaceManagerAction, Lease, MacAddr, ignore};

pub use crate::dhcp::client::DhcpClient;
use crate::{dhcp::client::DhcpClientError, interface::InterfaceManager};

const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_SERVER_PORT: u16 = 67;

pub struct DhcpManager {
    iface: String,
    mac: MacAddr,
    ifmgr: ActorRef<InterfaceManager>,
    client: Option<ActorRef<DhcpClient>>,
}

impl DhcpManager {
    pub fn new(
        iface: String,
        mac: MacAddr,
        ifmgr: ActorRef<InterfaceManager>,
    ) -> Self {
        Self {
            iface,
            mac,
            ifmgr,
            client: None,
        }
    }
}

impl Actor for DhcpManager {
    type Args = Self;
    type Error = DhcpClientError;

    async fn on_start(
        mut args: Self::Args,
        actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        let clinet =
            DhcpClient::new(args.iface.clone(), args.mac, actor_ref.clone())?;

        let client_ref =
            DhcpClient::spawn_with_mailbox(clinet, mailbox::unbounded());

        // Link client to supervisor
        actor_ref.link(&client_ref).await;
        args.client = Some(client_ref);

        Ok(args)
    }

    async fn on_link_died(
        &mut self,
        actor_ref: WeakActorRef<Self>,
        _id: ActorId,
        reason: ActorStopReason,
    ) -> Result<ControlFlow<ActorStopReason>, Self::Error> {
        // Restart the DHCP client after 15 seconds if it crashes
        if let Some(actor_ref) = actor_ref.upgrade() {
            tokio::time::sleep(Duration::from_secs(15)).await;
            let client = DhcpClient::new(
                self.iface.clone(),
                self.mac,
                actor_ref.clone(),
            )?;

            let client_ref =
                DhcpClient::spawn_with_mailbox(client, mailbox::unbounded());

            // Link client to supervisor
            actor_ref.link(&client_ref).await;
            self.client = Some(client_ref);

            Ok(ControlFlow::Continue(()))
        } else {
            Ok(ControlFlow::Break(reason))
        }
    }
}

impl Message<(Option<Lease>, Option<Lease>)> for DhcpManager {
    type Reply = ();

    async fn handle(
        &mut self,
        msg: (Option<Lease>, Option<Lease>),
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let (old_lease, new_lease) = msg;
        ignore!(
            self.ifmgr
                .tell(InterfaceManagerAction::DhcpV4Set {
                    name: self.iface.clone(),
                    old_lease,
                    new_lease,
                })
                .await
        );
    }
}
