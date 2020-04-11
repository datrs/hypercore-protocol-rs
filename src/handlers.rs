use crate::message::{ExtensionMessage, Message};
use crate::protocol::Protocol;
use crate::schema::*;
use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncWrite};
use std::io::{Error, Result};
use std::sync::Arc;

pub(crate) struct DefaultHandlers {}
#[async_trait]
impl StreamHandler for DefaultHandlers {}
impl DefaultHandlers {
    pub fn new() -> Arc<DefaultHandlers> {
        Arc::new(DefaultHandlers {})
    }
}

/// A type alias for [DynProtocol](DynProtocol) – this is the protocol handler you get
/// within [StreamHandler](StreamHandler) to send messages and open channels.
pub type StreamContext = (dyn DynProtocol);

/// A trait object wrapper for [Protocol](Protocol). These are the methods you can call
/// on the `context` parameter in [StreamHandler](StreamHandler) implementations.
/// You should not need to implement this yourself.
#[async_trait]
pub trait DynProtocol: Send {
    async fn send(&mut self, discovery_key: &[u8], message: Message) -> Result<()>;
    async fn open(&mut self, key: Vec<u8>, handlers: ChannelHandlerType) -> Result<()>;
    async fn destroy(&mut self, error: Error);
}

#[async_trait]
impl<R, W> DynProtocol for Protocol<R, W>
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    async fn send(&mut self, discovery_key: &[u8], message: Message) -> Result<()> {
        self.send_channel(discovery_key, message).await
    }

    async fn open(&mut self, key: Vec<u8>, handlers: ChannelHandlerType) -> Result<()> {
        self.open(key, handlers).await
    }

    async fn destroy(&mut self, error: Error) {
        self.destroy(error)
    }
}

/// The Channel struct is the `context` parameter being passed to
/// [ChannelHandler](ChannelHandler). It allows to send messages over the current channel,
/// open new channels, or destroy the Protocol.
pub struct Channel<'a> {
    protocol: &'a mut dyn DynProtocol,
    discovery_key: &'a [u8],
}

impl<'a> Channel<'a> {
    pub fn new(protocol: &'a mut dyn DynProtocol, discovery_key: &'a [u8]) -> Self {
        Self {
            discovery_key,
            protocol,
        }
    }

    pub fn discovery_key(&self) -> &[u8] {
        self.discovery_key
    }

    pub async fn send(&mut self, message: Message) -> Result<()> {
        let discovery_key = self.discovery_key;
        self.protocol.send(discovery_key, message).await
    }

    pub async fn open(&mut self, key: Vec<u8>, handlers: ChannelHandlerType) -> Result<()> {
        self.protocol.open(key, handlers).await
    }

    pub async fn status(&mut self, msg: Status) -> Result<()> {
        self.send(Message::Status(msg)).await
    }

    pub async fn options(&mut self, msg: Options) -> Result<()> {
        self.send(Message::Options(msg)).await
    }

    pub async fn have(&mut self, msg: Have) -> Result<()> {
        self.send(Message::Have(msg)).await
    }

    pub async fn unhave(&mut self, msg: Unhave) -> Result<()> {
        self.send(Message::Unhave(msg)).await
    }

    pub async fn want(&mut self, msg: Want) -> Result<()> {
        self.send(Message::Want(msg)).await
    }

    pub async fn unwant(&mut self, msg: Unwant) -> Result<()> {
        self.send(Message::Unwant(msg)).await
    }

    pub async fn request(&mut self, msg: Request) -> Result<()> {
        self.send(Message::Request(msg)).await
    }

    pub async fn cancel(&mut self, msg: Cancel) -> Result<()> {
        self.send(Message::Cancel(msg)).await
    }

    pub async fn data(&mut self, msg: Data) -> Result<()> {
        self.send(Message::Data(msg)).await
    }

    pub async fn close(&mut self, _msg: Close) -> Result<()> {
        // TODO: Implement close.
        // self.protocol.close(Close).await
        Ok(())
    }

    pub async fn destroy(&mut self, error: Error) {
        // TODO: Do not destroy the full protoocl but just this channel.
        self.protocol.destroy(error).await
    }
}

pub type StreamHandlerType = Arc<dyn StreamHandler + Send + Sync>;
pub type ChannelHandlerType = Arc<dyn ChannelHandler + Send + Sync>;

/// Implement this trait on a struct to handle stream-level events.
///
/// Set the handler when building a protocol in [ProtocolBuilder.set_handlers](ProtocolBuilder::set_handlers).
///
/// Example (where `Feed` would implement [ChannelHandler](ChannelHandler)):
/// ```
/// struct FeedStore {
///     feeds: Vec<Arc<Feed>>,
/// }
///
/// #[async_trait]
/// impl StreamHandler for FeedStore {
///     async fn on_discoverykey(
///         &self,
///         protocol: &mut StreamContext,
///         discovery_key: &[u8],
///     ) -> Result<()> {
///         let feed = self
///             .feeds
///             .iter()
///             .find(|feed| feed.discovery_key == discovery_key);
///         if let Some(feed) = feed {
///             let key = feed.key.clone();
///             let feed_handler = Arc::clone(&feed);
///             protocol.open(key, feed_handler).await
///         }
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait StreamHandler: Sync {
    async fn on_discoverykey(
        &self,
        _protocol: &mut StreamContext,
        _discovery_key: &[u8],
    ) -> Result<()> {
        Ok(())
    }
}

/// Implement this trait on a struct to handle channel-level events.
///
/// All methods are optional. Pass the struct on which you
/// implemented this trait into [protocol.open](Protocol::open).
#[async_trait]
pub trait ChannelHandler: Send + Sync {
    async fn on_message(&self, mut channel: &'_ mut Channel<'_>, message: Message) -> Result<()> {
        match message {
            Message::Options(msg) => self.on_options(&mut channel, msg).await,
            Message::Status(msg) => self.on_status(&mut channel, msg).await,
            Message::Have(msg) => self.on_have(&mut channel, msg).await,
            Message::Unhave(msg) => self.on_unhave(&mut channel, msg).await,
            Message::Want(msg) => self.on_want(&mut channel, msg).await,
            Message::Unwant(msg) => self.on_unwant(&mut channel, msg).await,
            Message::Request(msg) => self.on_request(&mut channel, msg).await,
            Message::Cancel(msg) => self.on_cancel(&mut channel, msg).await,
            Message::Data(msg) => self.on_data(&mut channel, msg).await,
            Message::Extension(msg) => self.on_extension(&mut channel, msg).await,
            // Open and close is handled at the stream level.
            // Message::Open(msg) => self.on_open(&mut channel, msg),
            // Message::Close(msg) => self.on_close(&mut channel, msg).await,
            _ => Ok(()),
        }
    }

    async fn on_open(&self, _protocol: &mut Channel<'_>, _discovery_key: &[u8]) -> Result<()> {
        Ok(())
    }

    async fn on_status(&self, _channel: &mut Channel<'_>, _message: Status) -> Result<()> {
        Ok(())
    }
    async fn on_options(&self, _channel: &mut Channel<'_>, _message: Options) -> Result<()> {
        Ok(())
    }
    async fn on_have(&self, _channel: &mut Channel<'_>, _message: Have) -> Result<()> {
        Ok(())
    }
    async fn on_unhave(&self, _channel: &mut Channel<'_>, _message: Unhave) -> Result<()> {
        Ok(())
    }
    async fn on_want(&self, _channel: &mut Channel<'_>, _message: Want) -> Result<()> {
        Ok(())
    }
    async fn on_unwant(&self, _channel: &mut Channel<'_>, _message: Unwant) -> Result<()> {
        Ok(())
    }
    async fn on_request(&self, _channel: &mut Channel<'_>, _message: Request) -> Result<()> {
        Ok(())
    }
    async fn on_cancel(&self, _channel: &mut Channel<'_>, _message: Cancel) -> Result<()> {
        Ok(())
    }
    async fn on_data(&self, _channel: &mut Channel<'_>, _message: Data) -> Result<()> {
        Ok(())
    }
    async fn on_close(&self, _channel: &mut Channel<'_>, _message: Close) -> Result<()> {
        Ok(())
    }
    async fn on_extension(
        &self,
        _channel: &mut Channel<'_>,
        _message: ExtensionMessage,
    ) -> Result<()> {
        Ok(())
    }
}
