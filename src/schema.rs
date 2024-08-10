use hypercore::encoding::{CompactEncoding, EncodingError, HypercoreState, State};
use hypercore::{
    DataBlock, DataHash, DataSeek, DataUpgrade, Proof, RequestBlock, RequestSeek, RequestUpgrade,
};

/// Open message
#[derive(Debug, Clone, PartialEq)]
pub struct Open {
    /// Channel id to open
    pub channel: u64,
    /// Protocol name
    pub protocol: String,
    /// Hypercore discovery key
    pub discovery_key: Vec<u8>,
    /// Capability hash
    pub capability: Option<Vec<u8>>,
}

impl CompactEncoding<Open> for State {
    fn preencode(&mut self, value: &Open) -> Result<usize, EncodingError> {
        self.preencode(&value.channel)?;
        self.preencode(&value.protocol)?;
        self.preencode(&value.discovery_key)?;
        if value.capability.is_some() {
            self.add_end(1)?; // flags for future use
            self.preencode_fixed_32()?;
        }
        Ok(self.end())
    }

    fn encode(&mut self, value: &Open, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        self.encode(&value.channel, buffer)?;
        self.encode(&value.protocol, buffer)?;
        self.encode(&value.discovery_key, buffer)?;
        if let Some(capability) = &value.capability {
            self.add_start(1)?; // flags for future use
            self.encode_fixed_32(capability, buffer)?;
        }
        Ok(self.start())
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Open, EncodingError> {
        let channel: u64 = self.decode(buffer)?;
        let protocol: String = self.decode(buffer)?;
        let discovery_key: Vec<u8> = self.decode(buffer)?;
        let capability: Option<Vec<u8>> = if self.start() < self.end() {
            self.add_start(1)?; // flags for future use
            let capability: Vec<u8> = self.decode_fixed_32(buffer)?.to_vec();
            Some(capability)
        } else {
            None
        };
        Ok(Open {
            channel,
            protocol,
            discovery_key,
            capability,
        })
    }
}

/// Close message
#[derive(Debug, Clone, PartialEq)]
pub struct Close {
    /// Channel id to close
    pub channel: u64,
}

impl CompactEncoding<Close> for State {
    fn preencode(&mut self, value: &Close) -> Result<usize, EncodingError> {
        self.preencode(&value.channel)
    }

    fn encode(&mut self, value: &Close, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        self.encode(&value.channel, buffer)
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Close, EncodingError> {
        let channel: u64 = self.decode(buffer)?;
        Ok(Close { channel })
    }
}

/// Synchronize message. Type 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Synchronize {
    /// Fork id, set to 0 for an un-forked hypercore.
    pub fork: u64,
    /// Length of hypercore
    pub length: u64,
    /// Known length of the remote party, 0 for unknown.
    pub remote_length: u64,
    /// Downloading allowed
    pub downloading: bool,
    /// Uploading allowed
    pub uploading: bool,
    /// Upgrade possible
    pub can_upgrade: bool,
}

impl CompactEncoding<Synchronize> for State {
    fn preencode(&mut self, value: &Synchronize) -> Result<usize, EncodingError> {
        self.add_end(1)?; // flags
        self.preencode(&value.fork)?;
        self.preencode(&value.length)?;
        self.preencode(&value.remote_length)
    }

    fn encode(&mut self, value: &Synchronize, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        let mut flags: u8 = if value.can_upgrade { 1 } else { 0 };
        flags |= if value.uploading { 2 } else { 0 };
        flags |= if value.downloading { 4 } else { 0 };
        self.encode(&flags, buffer)?;
        self.encode(&value.fork, buffer)?;
        self.encode(&value.length, buffer)?;
        self.encode(&value.remote_length, buffer)
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Synchronize, EncodingError> {
        let flags: u8 = self.decode(buffer)?;
        let fork: u64 = self.decode(buffer)?;
        let length: u64 = self.decode(buffer)?;
        let remote_length: u64 = self.decode(buffer)?;
        let can_upgrade = flags & 1 != 0;
        let uploading = flags & 2 != 0;
        let downloading = flags & 4 != 0;
        Ok(Synchronize {
            fork,
            length,
            remote_length,
            can_upgrade,
            uploading,
            downloading,
        })
    }
}

/// Request message. Type 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Request id, will be returned with corresponding [Data]
    pub id: u64,
    /// Current fork, set to 0 for un-forked hypercore
    pub fork: u64,
    /// Request for data
    pub block: Option<RequestBlock>,
    /// Request hash
    pub hash: Option<RequestBlock>,
    /// Request seek
    pub seek: Option<RequestSeek>,
    /// Request upgrade
    pub upgrade: Option<RequestUpgrade>,
}

impl CompactEncoding<Request> for HypercoreState {
    fn preencode(&mut self, value: &Request) -> Result<usize, EncodingError> {
        self.add_end(1)?; // flags
        self.0.preencode(&value.id)?;
        self.0.preencode(&value.fork)?;
        if let Some(block) = &value.block {
            self.preencode(block)?;
        }
        if let Some(hash) = &value.hash {
            self.preencode(hash)?;
        }
        if let Some(seek) = &value.seek {
            self.preencode(seek)?;
        }
        if let Some(upgrade) = &value.upgrade {
            self.preencode(upgrade)?;
        }
        Ok(self.end())
    }

    fn encode(&mut self, value: &Request, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        let mut flags: u8 = if value.block.is_some() { 1 } else { 0 };
        flags |= if value.hash.is_some() { 2 } else { 0 };
        flags |= if value.seek.is_some() { 4 } else { 0 };
        flags |= if value.upgrade.is_some() { 8 } else { 0 };
        self.0.encode(&flags, buffer)?;
        self.0.encode(&value.id, buffer)?;
        self.0.encode(&value.fork, buffer)?;
        if let Some(block) = &value.block {
            self.encode(block, buffer)?;
        }
        if let Some(hash) = &value.hash {
            self.encode(hash, buffer)?;
        }
        if let Some(seek) = &value.seek {
            self.encode(seek, buffer)?;
        }
        if let Some(upgrade) = &value.upgrade {
            self.encode(upgrade, buffer)?;
        }
        Ok(self.start())
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Request, EncodingError> {
        let flags: u8 = self.0.decode(buffer)?;
        let id: u64 = self.0.decode(buffer)?;
        let fork: u64 = self.0.decode(buffer)?;
        let block: Option<RequestBlock> = if flags & 1 != 0 {
            Some(self.decode(buffer)?)
        } else {
            None
        };
        let hash: Option<RequestBlock> = if flags & 2 != 0 {
            Some(self.decode(buffer)?)
        } else {
            None
        };
        let seek: Option<RequestSeek> = if flags & 4 != 0 {
            Some(self.decode(buffer)?)
        } else {
            None
        };
        let upgrade: Option<RequestUpgrade> = if flags & 8 != 0 {
            Some(self.decode(buffer)?)
        } else {
            None
        };
        Ok(Request {
            id,
            fork,
            block,
            hash,
            seek,
            upgrade,
        })
    }
}

/// Cancel message for a [Request]. Type 2
#[derive(Debug, Clone, PartialEq)]
pub struct Cancel {
    /// Request to cancel, see field `id` in [Request]
    pub request: u64,
}

impl CompactEncoding<Cancel> for State {
    fn preencode(&mut self, value: &Cancel) -> Result<usize, EncodingError> {
        self.preencode(&value.request)
    }

    fn encode(&mut self, value: &Cancel, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        self.encode(&value.request, buffer)
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Cancel, EncodingError> {
        let request: u64 = self.decode(buffer)?;
        Ok(Cancel { request })
    }
}

/// Data message responding to received [Request]. Type 3.
#[derive(Debug, Clone, PartialEq)]
pub struct Data {
    /// Request this data is for, see field `id` in [Request]
    pub request: u64,
    /// Fork id, set to 0 for un-forked hypercore
    pub fork: u64,
    /// Response for block request
    pub block: Option<DataBlock>,
    /// Response for hash request
    pub hash: Option<DataHash>,
    /// Response for seek request
    pub seek: Option<DataSeek>,
    /// Response for upgrade request
    pub upgrade: Option<DataUpgrade>,
}

impl CompactEncoding<Data> for HypercoreState {
    fn preencode(&mut self, value: &Data) -> Result<usize, EncodingError> {
        self.add_end(1)?; // flags
        self.0.preencode(&value.request)?;
        self.0.preencode(&value.fork)?;
        if let Some(block) = &value.block {
            self.preencode(block)?;
        }
        if let Some(hash) = &value.hash {
            self.preencode(hash)?;
        }
        if let Some(seek) = &value.seek {
            self.preencode(seek)?;
        }
        if let Some(upgrade) = &value.upgrade {
            self.preencode(upgrade)?;
        }
        Ok(self.end())
    }

    fn encode(&mut self, value: &Data, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        let mut flags: u8 = if value.block.is_some() { 1 } else { 0 };
        flags |= if value.hash.is_some() { 2 } else { 0 };
        flags |= if value.seek.is_some() { 4 } else { 0 };
        flags |= if value.upgrade.is_some() { 8 } else { 0 };
        self.0.encode(&flags, buffer)?;
        self.0.encode(&value.request, buffer)?;
        self.0.encode(&value.fork, buffer)?;
        if let Some(block) = &value.block {
            self.encode(block, buffer)?;
        }
        if let Some(hash) = &value.hash {
            self.encode(hash, buffer)?;
        }
        if let Some(seek) = &value.seek {
            self.encode(seek, buffer)?;
        }
        if let Some(upgrade) = &value.upgrade {
            self.encode(upgrade, buffer)?;
        }
        Ok(self.start())
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Data, EncodingError> {
        let flags: u8 = self.0.decode(buffer)?;
        let request: u64 = self.0.decode(buffer)?;
        let fork: u64 = self.0.decode(buffer)?;
        let block: Option<DataBlock> = if flags & 1 != 0 {
            Some(self.decode(buffer)?)
        } else {
            None
        };
        let hash: Option<DataHash> = if flags & 2 != 0 {
            Some(self.decode(buffer)?)
        } else {
            None
        };
        let seek: Option<DataSeek> = if flags & 4 != 0 {
            Some(self.decode(buffer)?)
        } else {
            None
        };
        let upgrade: Option<DataUpgrade> = if flags & 8 != 0 {
            Some(self.decode(buffer)?)
        } else {
            None
        };
        Ok(Data {
            request,
            fork,
            block,
            hash,
            seek,
            upgrade,
        })
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

/// No data message. Type 4.
#[derive(Debug, Clone, PartialEq)]
pub struct NoData {
    /// Request this message is for, see field `id` in [Request]
    pub request: u64,
}

impl CompactEncoding<NoData> for State {
    fn preencode(&mut self, value: &NoData) -> Result<usize, EncodingError> {
        self.preencode(&value.request)
    }

    fn encode(&mut self, value: &NoData, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        self.encode(&value.request, buffer)
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<NoData, EncodingError> {
        let request: u64 = self.decode(buffer)?;
        Ok(NoData { request })
    }
}

/// Want message. Type 5.
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    /// Start index
    pub start: u64,
    /// Length
    pub length: u64,
}
impl CompactEncoding<Want> for State {
    fn preencode(&mut self, value: &Want) -> Result<usize, EncodingError> {
        self.preencode(&value.start)?;
        self.preencode(&value.length)
    }

    fn encode(&mut self, value: &Want, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        self.encode(&value.start, buffer)?;
        self.encode(&value.length, buffer)
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Want, EncodingError> {
        let start: u64 = self.decode(buffer)?;
        let length: u64 = self.decode(buffer)?;
        Ok(Want { start, length })
    }
}

/// Un-want message. Type 6.
#[derive(Debug, Clone, PartialEq)]
pub struct Unwant {
    /// Start index
    pub start: u64,
    /// Length
    pub length: u64,
}
impl CompactEncoding<Unwant> for State {
    fn preencode(&mut self, value: &Unwant) -> Result<usize, EncodingError> {
        self.preencode(&value.start)?;
        self.preencode(&value.length)
    }

    fn encode(&mut self, value: &Unwant, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        self.encode(&value.start, buffer)?;
        self.encode(&value.length, buffer)
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Unwant, EncodingError> {
        let start: u64 = self.decode(buffer)?;
        let length: u64 = self.decode(buffer)?;
        Ok(Unwant { start, length })
    }
}

/// Bitfield message. Type 7.
#[derive(Debug, Clone, PartialEq)]
pub struct Bitfield {
    /// Start index of bitfield
    pub start: u64,
    /// Bitfield in 32 bit chunks beginning from `start`
    pub bitfield: Vec<u32>,
}
impl CompactEncoding<Bitfield> for State {
    fn preencode(&mut self, value: &Bitfield) -> Result<usize, EncodingError> {
        self.preencode(&value.start)?;
        self.preencode(&value.bitfield)
    }

    fn encode(&mut self, value: &Bitfield, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        self.encode(&value.start, buffer)?;
        self.encode(&value.bitfield, buffer)
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Bitfield, EncodingError> {
        let start: u64 = self.decode(buffer)?;
        let bitfield: Vec<u32> = self.decode(buffer)?;
        Ok(Bitfield { start, bitfield })
    }
}

/// Range message. Type 8.
/// Sender sharing info about a range of contiguous blocks it has.
/// the range is from Range.start to (Range.start + Range.length)
#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    /// If true, notifies that data has been cleared from this range.
    /// If false, notifies existing data range.
    pub drop: bool,
    /// range starts at this index
    pub start: u64,
    /// length of the range
    pub length: u64,
}

impl CompactEncoding<Range> for State {
    fn preencode(&mut self, value: &Range) -> Result<usize, EncodingError> {
        self.add_end(1)?; // flags
        self.preencode(&value.start)?;
        if value.length != 1 {
            self.preencode(&value.length)?;
        }
        Ok(self.end())
    }

    fn encode(&mut self, value: &Range, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        let mut flags: u8 = if value.drop { 1 } else { 0 };
        flags |= if value.length == 1 { 2 } else { 0 };
        self.encode(&flags, buffer)?;
        self.encode(&value.start, buffer)?;
        if value.length != 1 {
            self.encode(&value.length, buffer)?;
        }
        Ok(self.end())
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Range, EncodingError> {
        let flags: u8 = self.decode(buffer)?;
        let start: u64 = self.decode(buffer)?;
        let drop = flags & 1 != 0;
        let length: u64 = if flags & 2 != 0 {
            1
        } else {
            self.decode(buffer)?
        };
        Ok(Range {
            drop,
            length,
            start,
        })
    }
}

/// Extension message. Type 9. Use this for custom messages in your application.
#[derive(Debug, Clone, PartialEq)]
pub struct Extension {
    /// Name of the custom message
    pub name: String,
    /// Message content, use empty vector for no data.
    pub message: Vec<u8>,
}
impl CompactEncoding<Extension> for State {
    fn preencode(&mut self, value: &Extension) -> Result<usize, EncodingError> {
        self.preencode(&value.name)?;
        self.preencode_raw_buffer(&value.message)
    }

    fn encode(&mut self, value: &Extension, buffer: &mut [u8]) -> Result<usize, EncodingError> {
        self.encode(&value.name, buffer)?;
        self.encode_raw_buffer(&value.message, buffer)
    }

    fn decode(&mut self, buffer: &[u8]) -> Result<Extension, EncodingError> {
        let name: String = self.decode(buffer)?;
        let message: Vec<u8> = self.decode_raw_buffer(buffer)?;
        Ok(Extension { name, message })
    }
}
