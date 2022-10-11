use hypercore::compact_encoding::{CompactEncoding, State};

/// type=0
#[derive(Debug, Clone, PartialEq)]
pub struct Open {
    pub channel: u64,
    pub protocol: String,
    pub discovery_key: std::vec::Vec<u8>,
    pub capability: ::std::option::Option<std::vec::Vec<u8>>,
}

impl CompactEncoding<Open> for State {
    fn preencode(&mut self, value: &Open) {
        self.preencode(&value.channel);
        self.preencode(&value.protocol);
        self.preencode(&value.discovery_key);
        if value.capability.is_some() {
            self.end += 1; // flags for future use
            self.preencode_fixed_32();
        }
        println!("OPEN PREENCODE {:?}", self);
    }

    fn encode(&mut self, value: &Open, buffer: &mut [u8]) {
        self.encode(&value.channel, buffer);
        self.encode(&value.protocol, buffer);
        self.encode(&value.discovery_key, buffer);
        if let Some(capability) = &value.capability {
            self.start += 1; // flags for future use
            self.encode_fixed_32(capability, buffer);
        }
        println!("OPEN ENCODE {:?}", self);
    }

    fn decode(&mut self, buffer: &[u8]) -> Open {
        let channel: u64 = self.decode(buffer);
        let protocol: String = self.decode(buffer);
        let discovery_key: Vec<u8> = self.decode(buffer);
        println!(
            "OPEN DECODE self={:?}, channel={}, discovery_key={:02X?}, protocol={}",
            self, channel, discovery_key, protocol
        );
        let capability: Option<Vec<u8>> = if self.start < self.end {
            self.start += 1; // flags for future use
            let capability: Vec<u8> = self.decode_fixed_32(buffer).to_vec();
            Some(capability)
        } else {
            None
        };
        Open {
            channel,
            protocol,
            discovery_key,
            capability,
        }
    }
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
