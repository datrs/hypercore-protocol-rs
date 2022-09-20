cfg_if::cfg_if! {
    if #[cfg(feature = "v10")] {
use anyhow::Result;
use async_std::net::TcpStream;
use async_std::sync::{Arc, Mutex};
use async_std::task;
use futures_lite::stream::StreamExt;
use hypercore::{Hypercore, Node, Proof, PublicKey, Signature, Storage};
use log::*;
use random_access_memory::RandomAccessMemory;
use random_access_storage::RandomAccess;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::env;
use std::fmt::Debug;

use hypercore_protocol::schema::*;
use hypercore_protocol::{discovery_key, Channel, Event, Message, ProtocolBuilder};

mod util;
use util::{tcp_client, tcp_server};

fn main() {
    util::init_logger();
    if env::args().count() < 3 {
        usage();
    }
    let mode = env::args().nth(1).unwrap();
    let port = env::args().nth(2).unwrap();
    let address = format!("127.0.0.1:{}", port);

    let key = env::args().nth(3);
    let key: Option<[u8; 32]> = key.map_or(None, |key| {
        Some(
            hex::decode(key)
                .expect("Key has to be a hex string")
                .try_into()
                .expect("Key has to be a 32 byte hex string"),
        )
    });

    println!("HERERERE");

    // TODO
}

/// Print usage and exit.
fn usage() {
    println!("usage: cargo run --example hypercore -- [client|server] [port] [key]");
    std::process::exit(1);
}
} else {
        fn main() {}
    }
}
