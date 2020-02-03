use async_std::net::{TcpListener, TcpStream};
use async_std::stream::StreamExt;
use async_std::task;
use async_trait::async_trait;
use std::env;
use std::io::{ErrorKind, Result};
use std::sync::Arc;

// use simple_hypercore_protocol::create_from_tcp_stream;
use simple_hypercore_protocol::handshake::handshake;
use simple_hypercore_protocol::schema;
use simple_hypercore_protocol::types::Proto;
use simple_hypercore_protocol::{discovery_key, Handlers, Message, Proto, Protocol};

fn usage() {
    println!("usage: cargo run --example basic -- [client|server] [port] [key]");
    std::process::exit(1);
}

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
    println!("Listening on {}", listener.local_addr()?);

    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let key = key.clone();
        let stream = stream?;
        let peer_addr = stream.peer_addr().unwrap();
        eprintln!("new connection from {}", peer_addr);
        task::spawn(async move {
            match onconnection(stream, false, key).await {
                Err(ref e) if e.kind() != ErrorKind::UnexpectedEof => {
                    eprintln!("connection closed from {} with error: {}", peer_addr, e);
                }
                Err(_) | Ok(()) => {
                    eprintln!("connection closed from {}", peer_addr);
                }
            }
        });
    }
    Ok(())
}

async fn tcp_client(address: String, key: Option<String>) -> Result<()> {
    let stream = TcpStream::connect(&address).await?;
    onconnection(stream, true, key).await?;
    Ok(())
}

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

struct Feeds {
    feeds: Vec<Feed>,
}
impl Feeds {
    pub fn new() -> Self {
        Self { feeds: vec![] }
    }

    pub fn add(&mut self, feed: Feed) {
        self.feeds.push(feed);
    }
}

#[async_trait]
impl Handlers for Feeds {
    fn ondiscoverykey(&self, discovery_key: &[u8]) -> Option<Vec<u8>> {
        eprintln!("RESOLVE {:x?}", discovery_key);
        match self.feeds.iter().find(|f| f.discovery_key == discovery_key) {
            Some(feed) => Some(feed.key.clone()),
            None => None,
        }
    }
    async fn onopen(&self, protocol: &mut Proto, discovery_key: &[u8]) -> Result<()> {
        eprintln!("ONOPEN!!!! {:x?}", &discovery_key);
        protocol
            .send(
                &discovery_key,
                Message::Want(schema::Want {
                    start: 0,
                    length: Some(1048576),
                }),
            )
            .await?;
        Ok(())
    }
}

async fn onconnection(stream: TcpStream, is_initiator: bool, key: Option<String>) -> Result<()> {
    let reader = stream.clone();
    let writer = stream.clone();

    let (reader, writer, handshake) = handshake(reader, writer, is_initiator).await?;

    eprintln!("handshake complete! now init hypercore protocol");

    let mut handlers = Feeds::new();
    if let Some(key) = key {
        handlers.add(Feed::new(hex::decode(key).unwrap()));
    }

    // let handler = Handler {}

    let mut protocol = Protocol::new(reader, writer, Some(handshake));

    protocol.set_handlers(Arc::new(handlers));

    protocol.listen().await?;

    Ok(())
}
