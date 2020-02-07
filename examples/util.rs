use async_std::net::{TcpListener, TcpStream};
use async_std::prelude::*;
use async_std::task;
use env_logger::Env;
use std::io::Result;

/// Init EnvLogger, logging info, warn and error messages to stdout.
pub fn init_logger() {
    env_logger::from_env(Env::default().default_filter_or("info")).init();
}

/// Log a result if it's an error.
pub fn log_if_error(result: &Result<()>) {
    if let Err(err) = result.as_ref() {
        log::error!("error: {}", err);
    }
}

/// A simple async TCP server that calls an async function for each incoming connection.
pub async fn tcp_server<F, C>(
    address: String,
    onconnection: impl Fn(TcpStream, bool, C) -> F + Send + Sync + Copy + 'static,
    context: C,
) -> Result<()>
where
    F: Future<Output = Result<()>> + Send,
    C: Clone + Send + 'static,
{
    let listener = TcpListener::bind(&address).await?;
    log::info!("listening on {}", listener.local_addr()?);
    let mut incoming = listener.incoming();
    while let Some(Ok(stream)) = incoming.next().await {
        let context = context.clone();
        let peer_addr = stream.peer_addr().unwrap();
        log::info!("new connection from {}", peer_addr);
        task::spawn(async move {
            let result = onconnection(stream, false, context).await;
            log_if_error(&result);
            log::info!("connection closed from {}", peer_addr);
        });
    }
    Ok(())
}

/// A simple async TCP client that calls an async function when connected.
pub async fn tcp_client<F, C>(
    address: String,
    onconnection: impl Fn(TcpStream, bool, C) -> F + Send + Sync + Copy + 'static,
    context: C,
) -> Result<()>
where
    F: Future<Output = Result<()>> + Send,
    C: Clone + Send + 'static,
{
    let stream = TcpStream::connect(&address).await?;
    log::info!("connected to {}", &address);
    onconnection(stream, true, context).await
}
