use kameo::{
    Actor,
    actor::ActorRef,
    message::StreamMessage,
    prelude::{Context, Message},
};
use libnetwork_daemon::{ensure, ignore};
use serde::Serialize;

pub struct Subscriber<T, R, W, F>
where
    T: Send + 'static,
    R: Send + Serialize + 'static,
    W: Actor + Message<String> + Send + 'static,
    F: Fn(T) -> R + Send + 'static,
{
    writer: ActorRef<W>,
    response_builder: F,
    _phantom: std::marker::PhantomData<T>,
}

impl<T, R, W, F> Subscriber<T, R, W, F>
where
    T: Send + 'static,
    R: Send + Serialize + 'static,
    W: Actor + Message<String> + Send + 'static,
    F: Fn(T) -> R + Send + 'static,
{
    pub fn new(writer: ActorRef<W>, response_builder: F) -> Self {
        Self {
            writer,
            response_builder,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T, R, W, F> Actor for Subscriber<T, R, W, F>
where
    T: Send + 'static,
    R: Send + Serialize + 'static,
    W: Actor + Message<String> + Send + 'static,
    F: Fn(T) -> R + Send + 'static,
{
    type Args = Self;
    type Error = ();

    async fn on_start(
        args: Self::Args,
        _actor_ref: ActorRef<Self>,
    ) -> Result<Self, Self::Error> {
        Ok(args)
    }
}

impl<T, R, W, F> Message<StreamMessage<T, (), ()>> for Subscriber<T, R, W, F>
where
    T: Send + 'static,
    R: Send + Serialize + 'static,
    W: Actor + Message<String> + Send + 'static,
    F: Fn(T) -> R + Send + 'static,
{
    type Reply = ();

    async fn handle(
        &mut self,
        msg: StreamMessage<T, (), ()>,
        _ctx: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        if let StreamMessage::Next(msg) = msg {
            let response =
                ensure!(serde_json::to_string(&(self.response_builder)(msg)));
            ignore!(self.writer.tell(response).await);
        }
    }
}
