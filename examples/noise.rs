use async_std::net::TcpStream;
use async_std::task;
use hypercore_protocol::handshake;
use std::env;
use std::io::Result;

mod util;
use util::{tcp_client, tcp_server};

fn usage() {
    println!("usage: cargo run --example noise -- [client|server] [port] [key]");
    std::process::exit(1);
}

fn main() {
    util::init_logger();
    let count = env::args().count();
    if count < 3 {
        usage();
    }
    let mode = env::args().nth(1).unwrap();
    let port = env::args().nth(2).unwrap();

    let address = format!("127.0.0.1:{}", port);

    task::block_on(async move {
        let result = match mode.as_ref() {
            "server" => tcp_server(address, onconnection, None).await,
            "client" => tcp_client(address, onconnection, None).await,
            _ => panic!(usage()),
        };
        util::log_if_error(&result);
    });
}

async fn onconnection(stream: TcpStream, is_initiator: bool, _context: Option<()>) -> Result<()> {
    let mut reader = stream.clone();
    let mut writer = stream.clone();
    match handshake(&mut reader, &mut writer, is_initiator).await {
        Ok(_result) => eprintln!("handshake completed successfully"),
        Err(e) => eprintln!("error {:?}", e),
    };
    Ok(())
}
