// TODO: Use v10 structs here

/// Sent as part of the noise protocol.
#[derive(Debug, Clone, PartialEq)]
pub struct NoisePayload {
    pub nonce: std::vec::Vec<u8>,
}
/// type=0
#[derive(Debug, Clone, PartialEq)]
pub struct Open {
    pub discovery_key: std::vec::Vec<u8>,
    pub capability: ::std::option::Option<std::vec::Vec<u8>>,
}
/// type=1, overall feed options. can be sent multiple times
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Should be sorted lexicographically
    pub extensions: ::std::vec::Vec<std::string::String>,
    /// Should all blocks be explicitly acknowledged?
    pub ack: ::std::option::Option<bool>,
}
/// type=2, message indicating state changes etc.
/// initial state for uploading/downloading is true
#[derive(Debug, Clone, PartialEq)]
pub struct Status {
    pub uploading: ::std::option::Option<bool>,
    pub downloading: ::std::option::Option<bool>,
}
/// type=3, what do we have?
#[derive(Debug, Clone, PartialEq)]
pub struct Have {
    pub start: u64,
    /// defaults to 1
    pub length: ::std::option::Option<u64>,
    pub bitfield: ::std::option::Option<std::vec::Vec<u8>>,
    /// when true, this Have message is an acknowledgement
    pub ack: ::std::option::Option<bool>,
}
/// type=4, what did we lose?
#[derive(Debug, Clone, PartialEq)]
pub struct Unhave {
    pub start: u64,
    /// defaults to 1
    pub length: ::std::option::Option<u64>,
}
/// type=5, what do we want? remote should start sending have messages in this range
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    pub start: u64,
    /// defaults to Infinity or feed.length (if not live)
    pub length: ::std::option::Option<u64>,
}
/// type=6, what don't we want anymore?
#[derive(Debug, Clone, PartialEq)]
pub struct Unwant {
    pub start: u64,
    /// defaults to Infinity or feed.length (if not live)
    pub length: ::std::option::Option<u64>,
}
/// type=7, ask for data
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub index: u64,
    pub bytes: ::std::option::Option<u64>,
    pub hash: ::std::option::Option<bool>,
    pub nodes: ::std::option::Option<u64>,
}
/// type=8, cancel a request
#[derive(Debug, Clone, PartialEq)]
pub struct Cancel {
    pub index: u64,
    pub bytes: ::std::option::Option<u64>,
    pub hash: ::std::option::Option<bool>,
}
/// type=9, get some data
#[derive(Debug, Clone, PartialEq)]
pub struct Data {
    pub index: u64,
    pub value: ::std::option::Option<std::vec::Vec<u8>>,
    pub nodes: ::std::vec::Vec<data::Node>,
    pub signature: ::std::option::Option<std::vec::Vec<u8>>,
}
pub mod data {
    #[derive(Debug, Clone, PartialEq)]
    pub struct Node {
        pub index: u64,
        pub hash: std::vec::Vec<u8>,
        pub size: u64,
    }
}
/// type=10, explicitly close a channel.
#[derive(Debug, Clone, PartialEq)]
pub struct Close {
    /// only send this if you did not do an open
    pub discovery_key: ::std::option::Option<std::vec::Vec<u8>>,
}
