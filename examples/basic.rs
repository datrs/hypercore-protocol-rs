use async_std::net::{TcpListener, TcpStream};
use async_std::stream::StreamExt;
use async_std::task;
use async_trait::async_trait;
use std::env;
use std::io::Result;
use std::sync::Arc;

use simple_hypercore_protocol::{
    discovery_key, schema, ChannelContext, ChannelHandlers, Message, Protocol, StreamContext,
    StreamHandlers,
};

/// Print usage and exit.
fn usage() {
    println!("usage: cargo run --example basic -- [client|server] [port] [key]");
    std::process::exit(1);
}

/// Our main function. Starts either a TCP server or TCP client.
fn main() {
    let count = env::args().count();
    if count < 3 {
        usage();
    }
    let mode = env::args().nth(1).unwrap();
    let port = env::args().nth(2).unwrap();
    let key = env::args().nth(3);

    let address = format!("127.0.0.1:{}", port);

    task::block_on(async move {
        let result = match mode.as_ref() {
            "server" => tcp_server(address, key).await,
            "client" => tcp_client(address, key).await,
            _ => panic!(usage()),
        };
        if let Err(e) = result {
            eprintln!("error: {}", e);
        }
    });
}

async fn tcp_server(address: String, key: Option<String>) -> Result<()> {
    let listener = TcpListener::bind(&address).await?;
    println!("listening on {}", listener.local_addr()?);
    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        let key = key.clone();
        let peer_addr = stream.peer_addr().unwrap();
        eprintln!("new connection from {}", peer_addr);
        task::spawn(async move {
            let result = onconnection(stream, false, key).await;
            eprintln!(
                "connection closed from {} (error: {})",
                peer_addr,
                result
                    .err()
                    .map_or_else(|| "none".into(), |e| e.to_string())
            );
        });
    }
    Ok(())
}

async fn tcp_client(address: String, key: Option<String>) -> Result<()> {
    let stream = TcpStream::connect(&address).await?;
    onconnection(stream, true, key).await
}

// This is where we start our application code.

struct Feed {
    key: Vec<u8>,
    discovery_key: Vec<u8>,
}
impl Feed {
    pub fn new(key: Vec<u8>) -> Self {
        Self {
            discovery_key: discovery_key(&key),
            key,
        }
    }
}

#[async_trait]
impl ChannelHandlers for Feed {
    async fn on_open<'a>(
        &self,
        context: &mut ChannelContext<'a>,
        _discovery_key: &[u8],
    ) -> Result<()> {
        eprintln!("onchannelopen!!!");
        context
            .send(Message::Want(schema::Want {
                start: 0,
                length: Some(1048576),
            }))
            .await?;
        context
            .send(Message::Request(schema::Request {
                index: 0,
                bytes: None,
                hash: None,
                nodes: None,
            }))
            .await?;
        Ok(())
    }

    async fn on_data<'a>(
        &self,
        _context: &mut ChannelContext<'a>,
        msg: schema::Data,
    ) -> Result<()> {
        eprintln!("DATA: {}", String::from_utf8(msg.value.unwrap()).unwrap());
        Ok(())
    }
}

struct Feeds {
    feeds: Vec<Arc<Feed>>,
}
impl Feeds {
    pub fn new() -> Self {
        Self { feeds: vec![] }
    }

    pub fn add(&mut self, feed: Feed) {
        self.feeds.push(Arc::new(feed));
    }
}

#[async_trait]
impl StreamHandlers for Feeds {
    async fn on_discoverykey(
        &self,
        protocol: &mut StreamContext,
        discovery_key: &[u8],
    ) -> Result<()> {
        eprintln!("RESOLVE {:x?}", discovery_key);
        let feed = self.feeds.iter().find(|f| f.discovery_key == discovery_key);
        match feed {
            None => Ok(()),
            Some(feed) => protocol.open(feed.key.clone(), feed.clone()).await,
        }
    }
}

async fn onconnection(stream: TcpStream, is_initiator: bool, key: Option<String>) -> Result<()> {
    let mut handlers = Feeds::new();
    if let Some(key) = key {
        handlers.add(Feed::new(hex::decode(key).unwrap()));
    }

    let reader = stream.clone();
    let writer = stream.clone();
    let mut protocol = Protocol::from_rw_with_handshake(reader, writer, is_initiator).await?;

    // This would need more type annotations.
    // let protocol = Protocol::from_stream_with_handshake(stream, is_initiator).await?;
    eprintln!("handshake complete! now start protocol");
    protocol.set_handlers(Arc::new(handlers));
    protocol.listen().await?;

    Ok(())
}
