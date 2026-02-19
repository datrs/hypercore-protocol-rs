#![expect(unused)]
use std::{
    io::{self},
    pin::Pin,
    task::{Context, Poll},
};

use futures::{
    Sink, Stream, StreamExt,
    channel::mpsc::{UnboundedReceiver as Receiver, UnboundedSender as Sender, unbounded},
};

#[derive(Debug)]
pub(crate) struct Io {
    receiver: Receiver<Vec<u8>>,
    sender: Sender<Vec<u8>>,
}

impl Default for Io {
    fn default() -> Self {
        let (sender, receiver) = unbounded();
        Self { sender, receiver }
    }
}

impl Stream for Io {
    type Item = Vec<u8>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.receiver).poll_next(cx)
    }
}

impl Sink<Vec<u8>> for Io {
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(mut self: Pin<&mut Self>, item: Vec<u8>) -> Result<(), Self::Error> {
        Pin::new(&mut self.sender)
            .start_send(item)
            .map_err(|_e| io::Error::other("SendError"))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }
}

#[derive(Default, Debug)]
pub(crate) struct TwoWay {
    l_to_r: Io,
    r_to_l: Io,
}

impl TwoWay {
    fn split_sides(self) -> (Io, Io) {
        let left = Io {
            sender: self.l_to_r.sender,
            receiver: self.r_to_l.receiver,
        };
        let right = Io {
            sender: self.r_to_l.sender,
            receiver: self.l_to_r.receiver,
        };
        (left, right)
    }
}

pub(crate) fn log() {
    static START_LOGS: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    START_LOGS.get_or_init(|| {
        use tracing_subscriber::{
            EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _,
        };
        let env_filter = EnvFilter::from_default_env(); // Reads `RUST_LOG` environment variable

        // Create the hierarchical layer from tracing_tree
        let tree_layer = tracing_tree::HierarchicalLayer::new(2) // 2 spaces per indent level
            .with_targets(true)
            .with_bracketed_fields(true)
            .with_indent_lines(true)
            .with_thread_ids(false)
            //.with_thread_names(true)
            //.with_span_modes(true)
            ;

        tracing_subscriber::registry()
            .with(env_filter)
            .with(tree_layer)
            .init();
    });
}

pub(crate) struct Moo<Rx, Tx> {
    receiver: Rx,
    sender: Tx,
}

impl<RxItem, RxChannel: Stream<Item = RxItem> + Unpin, Tx: Unpin> Stream for Moo<RxChannel, Tx> {
    type Item = RxItem;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        Pin::new(&mut this.receiver).poll_next(cx)
    }
}

impl<Rx: Unpin, TxItem: Unpin, TxChannel: Sink<TxItem> + Unpin> Sink<TxItem>
    for Moo<Rx, TxChannel>
{
    type Error = io::Error;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: TxItem) -> Result<(), Self::Error> {
        let this = self.get_mut();
        Pin::new(&mut this.sender)
            .start_send(item)
            .map_err(|_e| io::Error::other("SendError"))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        todo!()
    }
}

/// Creaee [`Moo`] from return value of [`unbounded`]
impl<Rx, Tx> From<(Tx, Rx)> for Moo<Rx, Tx> {
    fn from(value: (Tx, Rx)) -> Self {
        Moo {
            receiver: value.1,
            sender: value.0,
        }
    }
}

impl<RxLeft, TxLeft> Moo<RxLeft, TxLeft> {
    /// connect two [`Moo`]s
    fn connect<RxRight, TxRight>(
        self,
        other: Moo<RxRight, TxRight>,
    ) -> (Moo<RxLeft, TxRight>, Moo<RxRight, TxLeft>) {
        let left = Moo {
            receiver: self.receiver,
            sender: other.sender,
        };
        let right = Moo {
            receiver: other.receiver,
            sender: self.sender,
        };
        (left, right)
    }
}

fn result_channel() -> (Sender<Vec<u8>>, impl Stream<Item = io::Result<Vec<u8>>>) {
    let (tx, rx) = unbounded::<Vec<u8>>();
    (tx, rx.map(Ok))
}

#[expect(clippy::type_complexity)]
pub(crate) fn create_result_connected() -> (
    Moo<impl Stream<Item = io::Result<Vec<u8>>>, impl Sink<Vec<u8>>>,
    Moo<impl Stream<Item = io::Result<Vec<u8>>>, impl Sink<Vec<u8>>>,
) {
    let a = Moo::from(result_channel());
    let b = Moo::from(result_channel());
    a.connect(b)
}

#[cfg(test)]
mod test {
    #[tokio::test]
    async fn way_one() {
        use futures::{SinkExt, StreamExt};
        let mut a = super::Io::default();
        let _ = a.send(b"hello".into()).await;
        let Some(res) = a.next().await else { panic!() };
        assert_eq!(res, b"hello");
    }

    #[tokio::test]
    async fn split() {
        use futures::{SinkExt, StreamExt};
        let (mut left, mut right) = (super::TwoWay::default()).split_sides();
        left.send(b"hello".to_vec()).await.unwrap();
        let Some(res) = right.next().await else {
            panic!();
        };
        assert_eq!(res, b"hello");
    }
}
