fn main() {
    prost_build::compile_protos(&["src/schema.proto"], &["src/"]).unwrap();
}
