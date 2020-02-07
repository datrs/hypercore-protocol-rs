use crate::message::{ExtensionMessage, Message};
use crate::protocol::Protocol;
use crate::schema::*;
use async_trait::async_trait;
use futures::io::{AsyncRead, AsyncWrite};
use std::io::{Error, Result};
use std::sync::Arc;

pub(crate) struct DefaultHandlers {}
#[async_trait]
impl StreamHandlers for DefaultHandlers {}
impl DefaultHandlers {
    pub fn new() -> Arc<DefaultHandlers> {
        Arc::new(DefaultHandlers {})
    }
}

/// A type alias for [DynProtocol](DynProtocol) – this is the protocol handler you get
/// within [StreamHandlers](StreamHandlers) to send messages and open channels.
pub type StreamContext = (dyn DynProtocol);

/// A trait object wrapper for [Protocol](Protocol). These are the methods you can call
/// on the `context` parameter in [StreamHandlers](StreamHandlers) implementations.
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
        self.destroy(error).await
    }
}

/// The ChannelContext struct is the `context` parameter being passed to
/// [ChannelHandlers](ChannelHandlers). It allows to send messages over the current channel,
/// open new channels, or destroy the Protocol.
pub struct ChannelContext<'a> {
    protocol: &'a mut dyn DynProtocol,
    discovery_key: &'a [u8],
}

impl<'a> ChannelContext<'a> {
    pub fn new(protocol: &'a mut dyn DynProtocol, discovery_key: &'a [u8]) -> Self {
        Self {
            discovery_key: discovery_key,
            protocol,
        }
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
        self.protocol.destroy(error).await
    }
}

pub type StreamHandlerType = Arc<dyn StreamHandlers + Send + Sync>;
pub type ChannelHandlerType = Arc<dyn ChannelHandlers + Send + Sync>;

/// Implement this trait on a struct to handle stream-level events.
///
/// Set the handler when building a protocol in [ProtocolBuilder.set_handlers](ProtocolBuilder::set_handlers).
///
/// Example (where `Feed` would implement [ChannelHandlers](ChannelHandlers)):
/// ```
/// struct FeedStore {
///     feeds: Vec<Arc<Feed>>,
/// }
///
/// #[async_trait]
/// impl StreamHandlers for FeedStore {
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
pub trait StreamHandlers: Sync {
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
pub trait ChannelHandlers: Send + Sync {
    async fn onmessage<'a>(
        &self,
        mut context: &'a mut ChannelContext<'a>,
        message: Message,
    ) -> Result<()> {
        match message {
            Message::Options(msg) => self.on_options(&mut context, msg).await,
            Message::Status(msg) => self.on_status(&mut context, msg).await,
            Message::Have(msg) => self.on_have(&mut context, msg).await,
            Message::Unhave(msg) => self.on_unhave(&mut context, msg).await,
            Message::Want(msg) => self.on_want(&mut context, msg).await,
            Message::Unwant(msg) => self.on_unwant(&mut context, msg).await,
            Message::Request(msg) => self.on_request(&mut context, msg).await,
            Message::Cancel(msg) => self.on_cancel(&mut context, msg).await,
            Message::Data(msg) => self.on_data(&mut context, msg).await,
            Message::Extension(msg) => self.on_extension(&mut context, msg).await,
            // Open is handled at the stream level.
            // Message::Open(msg) => self.on_open(&mut context, msg),
            Message::Close(msg) => self.on_close(&mut context, msg).await,
            _ => Ok(()),
        }
    }

    async fn on_open<'a>(
        &self,
        _protocol: &mut ChannelContext<'a>,
        _discovery_key: &[u8],
    ) -> Result<()> {
        Ok(())
    }

    async fn on_status<'a>(
        &self,
        _context: &mut ChannelContext<'a>,
        _message: Status,
    ) -> Result<()> {
        Ok(())
    }
    async fn on_options<'a>(
        &self,
        _context: &mut ChannelContext<'a>,
        _message: Options,
    ) -> Result<()> {
        Ok(())
    }
    async fn on_have<'a>(&self, _context: &mut ChannelContext<'a>, _message: Have) -> Result<()> {
        Ok(())
    }
    async fn on_unhave<'a>(
        &self,
        _context: &mut ChannelContext<'a>,
        _message: Unhave,
    ) -> Result<()> {
        Ok(())
    }
    async fn on_want<'a>(&self, _context: &mut ChannelContext<'a>, _message: Want) -> Result<()> {
        Ok(())
    }
    async fn on_unwant<'a>(
        &self,
        _context: &mut ChannelContext<'a>,
        _message: Unwant,
    ) -> Result<()> {
        Ok(())
    }
    async fn on_request<'a>(
        &self,
        _context: &mut ChannelContext<'a>,
        _message: Request,
    ) -> Result<()> {
        Ok(())
    }
    async fn on_cancel<'a>(
        &self,
        _context: &mut ChannelContext<'a>,
        _message: Cancel,
    ) -> Result<()> {
        Ok(())
    }
    async fn on_data<'a>(&self, _context: &mut ChannelContext<'a>, _message: Data) -> Result<()> {
        Ok(())
    }
    async fn on_close<'a>(&self, _context: &mut ChannelContext<'a>, _message: Close) -> Result<()> {
        Ok(())
    }
    async fn on_extension<'a>(
        &self,
        _context: &mut ChannelContext<'a>,
        _message: ExtensionMessage,
    ) -> Result<()> {
        Ok(())
    }
}
