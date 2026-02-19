use compact_encoding::{
    CompactEncoding, EncodingError, map_decode, map_encode, sum_encoded_size, take_array,
    take_array_mut, write_array, write_slice,
};
use hypercore_schema::{
    DataBlock, DataHash, DataSeek, DataUpgrade, Proof, RequestBlock, RequestSeek, RequestUpgrade,
};
use tracing::instrument;

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

impl CompactEncoding for Open {
    #[instrument(skip_all, ret, err)]
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        let out = sum_encoded_size!(self.channel, self.protocol, self.discovery_key);
        if self.capability.is_some() {
            return Ok(
                out
                    + 1  // flags for future use
                    + 32, // TODO capabalilities buff should always be 32 bytes, but it's a vec
            );
        }
        Ok(out)
    }

    #[instrument(skip_all)]
    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        let rest = map_encode!(buffer, self.channel, self.protocol, self.discovery_key);
        if let Some(cap) = &self.capability {
            let (_, rest) = take_array_mut::<1>(rest)?;
            return write_slice(cap, rest);
        }
        Ok(rest)
    }

    #[instrument(skip_all, err)]
    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ((channel, protocol, discovery_key), rest) =
            map_decode!(buffer, [u64, String, Vec<u8>]);
        // NB: Open/Close are only sent alone in their own Frame. So we're done when there is no
        // more data
        let (capability, rest) = if !rest.is_empty() {
            let (_, rest) = take_array::<1>(rest)?;
            let (capability, rest) = take_array::<32>(rest)?;
            (Some(capability.to_vec()), rest)
        } else {
            (None, rest)
        };
        Ok((
            Self {
                channel,
                protocol,
                discovery_key,
                capability,
            },
            rest,
        ))
    }
}

/// Close message
#[derive(Debug, Clone, PartialEq)]
pub struct Close {
    /// Channel id to close
    pub channel: u64,
}

impl CompactEncoding for Close {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        self.channel.encoded_size()
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        self.channel.encode(buffer)
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let (channel, rest) = u64::decode(buffer)?;
        Ok((Self { channel }, rest))
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

impl CompactEncoding for Synchronize {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        Ok(1 + sum_encoded_size!(self.fork, self.length, self.remote_length))
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        let mut flags: u8 = if self.can_upgrade { 1 } else { 0 };
        flags |= if self.uploading { 2 } else { 0 };
        flags |= if self.downloading { 4 } else { 0 };
        let rest = write_array(&[flags], buffer)?;
        Ok(map_encode!(
            rest,
            self.fork,
            self.length,
            self.remote_length
        ))
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ([flags], rest) = take_array::<1>(buffer)?;
        let ((fork, length, remote_length), rest) = map_decode!(rest, [u64, u64, u64]);
        let can_upgrade = flags & 1 != 0;
        let uploading = flags & 2 != 0;
        let downloading = flags & 4 != 0;
        Ok((
            Synchronize {
                fork,
                length,
                remote_length,
                can_upgrade,
                uploading,
                downloading,
            },
            rest,
        ))
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
    // TODO what is this
    /// Request manifest
    pub manifest: bool,
    // TODO what is this
    // this could prob be usize
    /// Request priority
    pub priority: u64,
}

macro_rules! maybe_decode {
    ($cond:expr, $type:ty, $buf:ident) => {
        if $cond {
            let (result, rest) = <$type>::decode($buf)?;
            (Some(result), rest)
        } else {
            (None, $buf)
        }
    };
}

impl CompactEncoding for Request {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        let mut out = 1; // flags
        out += sum_encoded_size!(self.id, self.fork);
        if let Some(block) = &self.block {
            out += block.encoded_size()?;
        }
        if let Some(hash) = &self.hash {
            out += hash.encoded_size()?;
        }
        if let Some(seek) = &self.seek {
            out += seek.encoded_size()?;
        }
        if let Some(upgrade) = &self.upgrade {
            out += upgrade.encoded_size()?;
        }
        if self.priority != 0 {
            out += self.priority.encoded_size()?;
        }
        Ok(out)
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        let mut flags: u8 = if self.block.is_some() { 1 } else { 0 };
        flags |= if self.hash.is_some() { 2 } else { 0 };
        flags |= if self.seek.is_some() { 4 } else { 0 };
        flags |= if self.upgrade.is_some() { 8 } else { 0 };
        flags |= if self.manifest { 16 } else { 0 };
        flags |= if self.priority != 0 { 32 } else { 0 };
        let mut rest = write_array(&[flags], buffer)?;
        rest = map_encode!(rest, self.id, self.fork);

        if let Some(block) = &self.block {
            rest = block.encode(rest)?;
        }
        if let Some(hash) = &self.hash {
            rest = hash.encode(rest)?;
        }
        if let Some(seek) = &self.seek {
            rest = seek.encode(rest)?;
        }
        if let Some(upgrade) = &self.upgrade {
            rest = upgrade.encode(rest)?;
        }

        if self.priority != 0 {
            rest = self.priority.encode(rest)?;
        }

        Ok(rest)
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ([flags], rest) = take_array::<1>(buffer)?;
        let ((id, fork), rest) = map_decode!(rest, [u64, u64]);

        let (block, rest) = maybe_decode!(flags & 1 != 0, RequestBlock, rest);
        let (hash, rest) = maybe_decode!(flags & 2 != 0, RequestBlock, rest);
        let (seek, rest) = maybe_decode!(flags & 4 != 0, RequestSeek, rest);
        let (upgrade, rest) = maybe_decode!(flags & 8 != 0, RequestUpgrade, rest);
        let manifest = flags & 16 != 0;
        let (priority, rest) = if flags & 32 != 0 {
            u64::decode(rest)?
        } else {
            (0, rest)
        };
        Ok((
            Request {
                id,
                fork,
                block,
                hash,
                seek,
                upgrade,
                manifest,
                priority,
            },
            rest,
        ))
    }
}

/// Cancel message for a [Request]. Type 2
#[derive(Debug, Clone, PartialEq)]
pub struct Cancel {
    /// Request to cancel, see field `id` in [Request]
    pub request: u64,
}

impl CompactEncoding for Cancel {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        self.request.encoded_size()
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        self.request.encode(buffer)
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let (request, rest) = u64::decode(buffer)?;
        Ok((Cancel { request }, rest))
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

macro_rules! opt_encoded_size {
    ($opt:expr, $sum:ident) => {
        if let Some(thing) = $opt {
            $sum += thing.encoded_size()?;
        }
    };
}

// TODO we could write a macro where it takes a $cond that returns an opt.
// if the option is Some(T) then do T::encode(buf)
// also if some add $flag.
// This would simplify some of these impls
macro_rules! opt_encoded_bytes {
    ($opt:expr, $buf:ident) => {
        if let Some(thing) = $opt {
            thing.encode($buf)?
        } else {
            $buf
        }
    };
}
impl CompactEncoding for Data {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        let mut out = 1; // flags
        out += sum_encoded_size!(self.request, self.fork);
        opt_encoded_size!(&self.block, out);
        opt_encoded_size!(&self.hash, out);
        opt_encoded_size!(&self.seek, out);
        opt_encoded_size!(&self.upgrade, out);
        Ok(out)
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        let mut flags: u8 = if self.block.is_some() { 1 } else { 0 };
        flags |= if self.hash.is_some() { 2 } else { 0 };
        flags |= if self.seek.is_some() { 4 } else { 0 };
        flags |= if self.upgrade.is_some() { 8 } else { 0 };
        let rest = write_array(&[flags], buffer)?;
        let rest = map_encode!(rest, self.request, self.fork);

        let rest = opt_encoded_bytes!(&self.block, rest);
        let rest = opt_encoded_bytes!(&self.hash, rest);
        let rest = opt_encoded_bytes!(&self.seek, rest);
        let rest = opt_encoded_bytes!(&self.upgrade, rest);
        Ok(rest)
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ([flags], rest) = take_array::<1>(buffer)?;
        let ((request, fork), rest) = map_decode!(rest, [u64, u64]);
        let (block, rest) = maybe_decode!(flags & 1 != 0, DataBlock, rest);
        let (hash, rest) = maybe_decode!(flags & 2 != 0, DataHash, rest);
        let (seek, rest) = maybe_decode!(flags & 4 != 0, DataSeek, rest);
        let (upgrade, rest) = maybe_decode!(flags & 8 != 0, DataUpgrade, rest);
        Ok((
            Data {
                request,
                fork,
                block,
                hash,
                seek,
                upgrade,
            },
            rest,
        ))
    }
}

impl Data {
    /// Transform Data message into a [`Proof`]
    pub fn into_proof(self) -> Proof {
        Proof {
            fork: self.fork,
            block: self.block,
            hash: self.hash,
            seek: self.seek,
            upgrade: self.upgrade,
        }
    }
}

/// No data message. Type 4.
#[derive(Debug, Clone, PartialEq)]
pub struct NoData {
    /// Request this message is for, see field `id` in [Request]
    pub request: u64,
}

impl CompactEncoding for NoData {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        Ok(sum_encoded_size!(self.request))
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        Ok(map_encode!(buffer, self.request))
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let (request, rest) = u64::decode(buffer)?;
        Ok((Self { request }, rest))
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

impl CompactEncoding for Want {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        Ok(sum_encoded_size!(self.start, self.length))
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        Ok(map_encode!(buffer, self.start, self.length))
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ((start, length), rest) = map_decode!(buffer, [u64, u64]);
        Ok((Self { start, length }, rest))
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

impl CompactEncoding for Unwant {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        Ok(sum_encoded_size!(self.start, self.length))
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        Ok(map_encode!(buffer, self.start, self.length))
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ((start, length), rest) = map_decode!(buffer, [u64, u64]);
        Ok((Self { start, length }, rest))
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
impl CompactEncoding for Bitfield {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        Ok(sum_encoded_size!(self.start, self.bitfield))
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        Ok(map_encode!(buffer, self.start, self.bitfield))
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ((start, bitfield), rest) = map_decode!(buffer, [u64, Vec<u32>]);
        Ok((Self { start, bitfield }, rest))
    }
}

/// Range message. Type 8.
/// Notifies Peer's that the Sender has a range of contiguous blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    /// If true, notifies that data has been cleared from this range.
    /// If false, notifies existing data range.
    pub drop: bool,
    /// Range starts at this index
    pub start: u64,
    /// Length of the range
    pub length: u64,
}

impl CompactEncoding for Range {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        let mut out = 1 + sum_encoded_size!(self.start);
        if self.length != 1 {
            out += self.length.encoded_size()?;
        }
        Ok(out)
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        let mut flags: u8 = if self.drop { 1 } else { 0 };
        flags |= if self.length == 1 { 2 } else { 0 };
        let rest = write_array(&[flags], buffer)?;
        let rest = self.start.encode(rest)?;
        if self.length != 1 {
            return self.length.encode(rest);
        }
        Ok(rest)
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ([flags], rest) = take_array::<1>(buffer)?;
        let (start, rest) = u64::decode(rest)?;
        let drop = flags & 1 != 0;
        let (length, rest) = if flags & 2 != 0 {
            (1, rest)
        } else {
            u64::decode(rest)?
        };
        Ok((
            Range {
                drop,
                length,
                start,
            },
            rest,
        ))
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
impl CompactEncoding for Extension {
    fn encoded_size(&self) -> Result<usize, EncodingError> {
        Ok(sum_encoded_size!(self.name, self.message))
    }

    fn encode<'a>(&self, buffer: &'a mut [u8]) -> Result<&'a mut [u8], EncodingError> {
        Ok(map_encode!(buffer, self.name, self.message))
    }

    fn decode(buffer: &[u8]) -> Result<(Self, &[u8]), EncodingError>
    where
        Self: Sized,
    {
        let ((name, message), rest) = map_decode!(buffer, [String, Vec<u8>]);
        Ok((Self { name, message }, rest))
    }
}
