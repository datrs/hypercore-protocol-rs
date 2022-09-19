#[cfg(feature = "v9")]
fn main() {
    prost_build::compile_protos(&["src/schema.proto"], &["src/"]).unwrap();
}

#[cfg(feature = "v10")]
fn main() {}
