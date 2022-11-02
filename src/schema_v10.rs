use hypercore::compact_encoding::{CompactEncoding, State};
use hypercore::{
    DataBlock, DataHash, DataSeek, DataUpgrade, Proof, RequestBlock, RequestSeek, RequestUpgrade,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Open {
    pub channel: u64,
    pub protocol: String,
    pub discovery_key: Vec<u8>,
    pub capability: Option<Vec<u8>>,
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
    }

    fn encode(&mut self, value: &Open, buffer: &mut [u8]) {
        self.encode(&value.channel, buffer);
        self.encode(&value.protocol, buffer);
        self.encode(&value.discovery_key, buffer);
        if let Some(capability) = &value.capability {
            self.start += 1; // flags for future use
            self.encode_fixed_32(capability, buffer);
        }
    }

    fn decode(&mut self, buffer: &[u8]) -> Open {
        let channel: u64 = self.decode(buffer);
        let protocol: String = self.decode(buffer);
        let discovery_key: Vec<u8> = self.decode(buffer);
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

#[derive(Debug, Clone, PartialEq)]
pub struct Close {
    pub discovery_key: Option<Vec<u8>>,
}

impl CompactEncoding<Close> for State {
    fn preencode(&mut self, value: &Close) {
        if let Some(discovery_key) = value.discovery_key.as_ref() {
            self.preencode(discovery_key);
        }
    }

    fn encode(&mut self, value: &Close, buffer: &mut [u8]) {
        if let Some(discovery_key) = value.discovery_key.as_ref() {
            self.encode(discovery_key, buffer);
        }
    }

    fn decode(&mut self, buffer: &[u8]) -> Close {
        let discovery_key: Option<Vec<u8>> = if buffer.len() > 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        Close { discovery_key }
    }
}

/// Type 0
#[derive(Debug, Clone, PartialEq)]
pub struct Synchronize {
    pub fork: u64,
    pub length: u64,
    pub remote_length: u64,
    pub downloading: bool,
    pub uploading: bool,
    pub can_upgrade: bool,
}

impl CompactEncoding<Synchronize> for State {
    fn preencode(&mut self, value: &Synchronize) {
        self.end += 1; // flags
        self.preencode(&value.fork);
        self.preencode(&value.length);
        self.preencode(&value.remote_length);
    }

    fn encode(&mut self, value: &Synchronize, buffer: &mut [u8]) {
        let mut flags: u8 = if value.can_upgrade { 1 } else { 0 };
        flags = flags | if value.uploading { 2 } else { 0 };
        flags = flags | if value.downloading { 4 } else { 0 };
        self.encode(&flags, buffer);
        self.encode(&value.fork, buffer);
        self.encode(&value.length, buffer);
        self.encode(&value.remote_length, buffer);
    }

    fn decode(&mut self, buffer: &[u8]) -> Synchronize {
        let flags: u8 = self.decode(buffer);
        let fork: u64 = self.decode(buffer);
        let length: u64 = self.decode(buffer);
        let remote_length: u64 = self.decode(buffer);
        let can_upgrade = flags & 1 != 0;
        let uploading = flags & 2 != 0;
        let downloading = flags & 4 != 0;
        Synchronize {
            fork,
            length,
            remote_length,
            can_upgrade,
            uploading,
            downloading,
        }
    }
}

/// Type 1: Request.
/// Contains sub structs
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: u64,
    pub fork: u64,
    pub block: Option<RequestBlock>,
    pub hash: Option<RequestBlock>,
    pub seek: Option<RequestSeek>,
    pub upgrade: Option<RequestUpgrade>,
}

impl CompactEncoding<Request> for State {
    fn preencode(&mut self, value: &Request) {
        self.end += 1; // flags
        self.preencode(&value.id);
        self.preencode(&value.fork);
        if let Some(block) = &value.block {
            self.preencode(block);
        }
        if let Some(hash) = &value.hash {
            self.preencode(hash);
        }
        if let Some(seek) = &value.seek {
            self.preencode(seek);
        }
        if let Some(upgrade) = &value.upgrade {
            self.preencode(upgrade);
        }
    }

    fn encode(&mut self, value: &Request, buffer: &mut [u8]) {
        let mut flags: u8 = if value.block.is_some() { 1 } else { 0 };
        flags = flags | if value.hash.is_some() { 2 } else { 0 };
        flags = flags | if value.seek.is_some() { 4 } else { 0 };
        flags = flags | if value.upgrade.is_some() { 8 } else { 0 };
        self.encode(&flags, buffer);
        self.encode(&value.id, buffer);
        self.encode(&value.fork, buffer);
        if let Some(block) = &value.block {
            self.encode(block, buffer);
        }
        if let Some(hash) = &value.hash {
            self.encode(hash, buffer);
        }
        if let Some(seek) = &value.seek {
            self.encode(seek, buffer);
        }
        if let Some(upgrade) = &value.upgrade {
            self.encode(upgrade, buffer);
        }
    }

    fn decode(&mut self, buffer: &[u8]) -> Request {
        let flags: u8 = self.decode(buffer);
        let id: u64 = self.decode(buffer);
        let fork: u64 = self.decode(buffer);
        let block: Option<RequestBlock> = if flags & 1 != 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        let hash: Option<RequestBlock> = if flags & 2 != 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        let seek: Option<RequestSeek> = if flags & 4 != 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        let upgrade: Option<RequestUpgrade> = if flags & 8 != 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        Request {
            id,
            fork,
            block,
            hash,
            seek,
            upgrade,
        }
    }
}

/// Type 2, Cancel a request based on its id
#[derive(Debug, Clone, PartialEq)]
pub struct Cancel {
    pub request: u64,
}

impl CompactEncoding<Cancel> for State {
    fn preencode(&mut self, value: &Cancel) {
        self.preencode(&value.request);
    }

    fn encode(&mut self, value: &Cancel, buffer: &mut [u8]) {
        self.encode(&value.request, buffer);
    }

    fn decode(&mut self, buffer: &[u8]) -> Cancel {
        let request: u64 = self.decode(buffer);
        Cancel { request }
    }
}

/// Type 3: Data.
/// Contains sub structs
#[derive(Debug, Clone, PartialEq)]
pub struct Data {
    pub request: u64,
    pub fork: u64,
    pub block: Option<DataBlock>,
    pub hash: Option<DataHash>,
    pub seek: Option<DataSeek>,
    pub upgrade: Option<DataUpgrade>,
}

impl CompactEncoding<Data> for State {
    fn preencode(&mut self, value: &Data) {
        self.end += 1; // flags
        self.preencode(&value.request);
        self.preencode(&value.fork);
        if let Some(block) = &value.block {
            self.preencode(block);
        }
        if let Some(hash) = &value.hash {
            self.preencode(hash);
        }
        if let Some(seek) = &value.seek {
            self.preencode(seek);
        }
        if let Some(upgrade) = &value.upgrade {
            self.preencode(upgrade);
        }
    }

    fn encode(&mut self, value: &Data, buffer: &mut [u8]) {
        let mut flags: u8 = if value.block.is_some() { 1 } else { 0 };
        flags = flags | if value.hash.is_some() { 2 } else { 0 };
        flags = flags | if value.seek.is_some() { 4 } else { 0 };
        flags = flags | if value.upgrade.is_some() { 8 } else { 0 };
        self.encode(&flags, buffer);
        self.encode(&value.request, buffer);
        self.encode(&value.fork, buffer);
        if let Some(block) = &value.block {
            self.encode(block, buffer);
        }
        if let Some(hash) = &value.hash {
            self.encode(hash, buffer);
        }
        if let Some(seek) = &value.seek {
            self.encode(seek, buffer);
        }
        if let Some(upgrade) = &value.upgrade {
            self.encode(upgrade, buffer);
        }
    }

    fn decode(&mut self, buffer: &[u8]) -> Data {
        let flags: u8 = self.decode(buffer);
        let request: u64 = self.decode(buffer);
        let fork: u64 = self.decode(buffer);
        let block: Option<DataBlock> = if flags & 1 != 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        let hash: Option<DataHash> = if flags & 2 != 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        let seek: Option<DataSeek> = if flags & 4 != 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        let upgrade: Option<DataUpgrade> = if flags & 8 != 0 {
            Some(self.decode(buffer))
        } else {
            None
        };
        Data {
            request,
            fork,
            block,
            hash,
            seek,
            upgrade,
        }
    }
}

impl Data {
    /// Transform Data message into a Proof emptying fields
    pub fn into_proof(&mut self) -> Proof {
        Proof {
            fork: self.fork,
            block: self.block.take(),
            hash: self.hash.take(),
            seek: self.seek.take(),
            upgrade: self.upgrade.take(),
        }
    }
}

/// Type 4, No data available for given request id
#[derive(Debug, Clone, PartialEq)]
pub struct NoData {
    pub request: u64,
}

impl CompactEncoding<NoData> for State {
    fn preencode(&mut self, value: &NoData) {
        self.preencode(&value.request);
    }

    fn encode(&mut self, value: &NoData, buffer: &mut [u8]) {
        self.encode(&value.request, buffer);
    }

    fn decode(&mut self, buffer: &[u8]) -> NoData {
        let request: u64 = self.decode(buffer);
        NoData { request }
    }
}

/// Type 5, Want
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    pub start: u64,
    pub length: u64,
}
impl CompactEncoding<Want> for State {
    fn preencode(&mut self, value: &Want) {
        self.preencode(&value.start);
        self.preencode(&value.length);
    }

    fn encode(&mut self, value: &Want, buffer: &mut [u8]) {
        self.encode(&value.start, buffer);
        self.encode(&value.length, buffer);
    }

    fn decode(&mut self, buffer: &[u8]) -> Want {
        let start: u64 = self.decode(buffer);
        let length: u64 = self.decode(buffer);
        Want { start, length }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    pub drop: bool,
    pub start: u64,
    pub length: u64,
}

impl CompactEncoding<Range> for State {
    fn preencode(&mut self, value: &Range) {
        self.end += 1; // flags
        self.preencode(&value.start);
        if value.length != 1 {
            self.preencode(&value.length);
        }
    }

    fn encode(&mut self, value: &Range, buffer: &mut [u8]) {
        let mut flags: u8 = if value.drop { 1 } else { 0 };
        flags = flags | if value.length == 1 { 2 } else { 0 };
        self.encode(&flags, buffer);
        self.encode(&value.start, buffer);
        if value.length != 1 {
            self.encode(&value.length, buffer);
        }
    }

    fn decode(&mut self, buffer: &[u8]) -> Range {
        let flags: u8 = self.decode(buffer);
        let start: u64 = self.decode(buffer);
        let drop = flags & 1 != 0;
        let length: u64 = if flags & 2 != 0 {
            1
        } else {
            self.decode(buffer)
        };
        Range {
            drop,
            length,
            start,
        }
    }
}

/// TODO: Remove this legacy stuff below

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

/// type=6, what don't we want anymore?
#[derive(Debug, Clone, PartialEq)]
pub struct Unwant {
    pub start: u64,
    /// defaults to Infinity or feed.length (if not live)
    pub length: ::std::option::Option<u64>,
}
