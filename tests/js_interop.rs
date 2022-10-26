#![cfg(feature = "v10")]

use anyhow::Result;
use async_std::io::prelude::BufReadExt;
use instant::Duration;
use std::{path::Path, sync::Once};

mod js;

use js::{cleanup, install, js_run_client, js_start_server, prepare_test_set};

static INIT: Once = Once::new();
fn init() {
    INIT.call_once(|| {
        // run initialization here
        cleanup();
        install();
    });
}

const TEST_SET_NODE_CLIENT_NODE_SERVER: &str = "ncns";
const TEST_SET_SERVER_WRITER: &str = "sw";
const TEST_SET_CLIENT_WRITER: &str = "cw";

#[async_std::test]
#[cfg_attr(not(feature = "js_interop_tests"), ignore)]
async fn js_interop_ncns_sw_simple() -> Result<()> {
    init();
    let test_set = format!(
        "{}_{}_{}",
        TEST_SET_NODE_CLIENT_NODE_SERVER, TEST_SET_SERVER_WRITER, "simple"
    );
    let server_is_writer = true;
    let (result_path, reader_path, writer_path) = prepare_test_set(&test_set);
    let port = 4000;
    let item_count = 4;
    let item_size = 4;
    let data_char = '1';
    let _server = js_start_server(
        server_is_writer,
        port,
        item_count,
        item_size,
        data_char,
        test_set.clone(),
    )
    .await?;
    js_run_client(
        !server_is_writer,
        port,
        item_count,
        item_size,
        data_char,
        &test_set,
    )
    .await;
    assert_result(result_path, item_count, item_size, data_char).await?;

    Ok(())
}

async fn assert_result(
    result_path: String,
    item_count: usize,
    item_size: usize,
    data_char: char,
) -> Result<()> {
    let mut reader = async_std::io::BufReader::new(async_std::fs::File::open(result_path).await?);

    let mut i: usize = 0;
    let expected_value = data_char.to_string().repeat(item_size);
    let mut line = String::new();
    while reader.read_line(&mut line).await? != 0 {
        assert_eq!(line, format!("{} {}\n", i, expected_value));
        i += 1;
        line = String::new();
    }
    assert_eq!(i, item_count);
    Ok(())
}
