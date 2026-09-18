//! Bounded decoders for Source server-to-client and client-to-server message streams.
//!
//! Message IDs and field widths match `common/protocol.h` and
//! `common/netmessages.cpp`. Protocol 25 is the strict default; older demo
//! network protocols can be inspected explicitly through [`ParseOptions`].

use source_binary::{BitReader, BitWriter};
use std::collections::HashMap;
use std::fmt;
use std::ops::Range;
use std::sync::OnceLock;

pub const NETWORK_PROTOCOL: i32 = 25;
pub const MESSAGE_TYPE_BITS: u32 = 6;

/// `NET_HEADER_FLAG_SPLITPACKET`, the leading word that marks a datagram as
/// one piece of a larger one.
pub const SPLIT_PACKET_FLAG: i32 = -2;
/// The packed `SPLITPACKET` header: two 32-bit words then two 16-bit fields.
pub const SPLIT_PACKET_HEADER_BYTES: usize = 12;
/// `MAX_ROUTABLE_PAYLOAD` less the header, the largest piece a sender may cut.
pub const MAX_SPLIT_PAYLOAD_BYTES: usize = 1260 - SPLIT_PACKET_HEADER_BYTES;
/// `MIN_USER_MAXROUTABLE_SIZE` less the header, from the X.25 floor of 576.
pub const MIN_SPLIT_PAYLOAD_BYTES: usize = 576 - SPLIT_PACKET_HEADER_BYTES;
/// `NET_MAX_MESSAGE`, which is `NET_MAX_PAYLOAD` plus the nine header bytes
/// padded up to sixteen.
pub const MAX_REASSEMBLED_BYTES: usize = 288_016;
/// `MAX_SPLITPACKET_SPLITS`, how many pieces the smallest split size needs to
/// carry a whole message.
pub const MAX_SPLIT_COUNT: usize = MAX_REASSEMBLED_BYTES / MIN_SPLIT_PAYLOAD_BYTES;

pub const PACKET_ACCEPTED: u32 = 0;
pub const PACKET_DUPLICATE: u32 = 1;
pub const PACKET_OUT_OF_ORDER: u32 = 2;
pub const PACKET_EXCESSIVE_DROP: u32 = 3;
pub const PACKET_HEADER_ACCEPTED: u32 = 0;
pub const PACKET_HEADER_TRUNCATED: u32 = 1;
pub const PACKET_HEADER_CHECKSUM_MISMATCH: u32 = 2;
pub const PACKET_HEADER_CHALLENGE_MISMATCH: u32 = 3;
pub const PACKET_HEADER_CHALLENGE_MISSING: u32 = 4;
pub const PACKET_FLAG_CHOKED: u8 = 1 << 4;
pub const PACKET_FLAG_CHALLENGE: u8 = 1 << 5;
pub const MAX_ENCODED_PACKET_HEADER_BYTES: usize = 17;
pub const MAX_NETWORK_STRING_BYTES: usize = 4096;
pub const MAX_NETWORK_USER_DATA_BYTES: usize = (1 << 14) - 1;
pub const MAX_DATA_TABLES: usize = 1024;
pub const MAX_DATA_TABLE_PROPERTIES: usize = (1 << 10) - 1;
pub const MAX_SERVER_CLASSES: usize = 1 << 9;
pub const MAX_DATA_TABLE_NAME_BYTES: usize = 512;
pub const SEND_PROP_EXCLUDE: u32 = 1 << 6;
pub const SEND_PROP_FLAG_MASK: u32 = (1 << 17) - 1;
pub const SEND_PROP_ARRAY: u32 = 5;
pub const SEND_PROP_DATA_TABLE: u32 = 6;
pub const MAX_SNAPSHOT_ENTITIES: usize = 1 << 11;
pub const MAX_ACTIVE_SNAPSHOTS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketDecision {
    pub incoming_sequence: i32,
    pub outgoing_ack: i32,
    pub dropped: i32,
    pub accepted: bool,
    pub reason: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SequenceAdvance {
    pub previous: i32,
    pub current: i32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PacketHeaderDecision {
    pub sequence: i32,
    pub outgoing_ack: i32,
    pub flags: u8,
    pub reliable_state: u8,
    pub choked: u8,
    pub challenge: u32,
    pub header_bytes: usize,
    pub accepted: bool,
    pub reason: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncodedPacketHeader {
    pub bytes: [u8; MAX_ENCODED_PACKET_HEADER_BYTES],
    pub length: usize,
    pub flags_offset: usize,
    pub checksum_offset: Option<usize>,
    pub checksum_start: usize,
    pub base_flags: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketHeaderError {
    Truncated(usize),
}

impl fmt::Display for PacketHeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated(length) => {
                write!(formatter, "packet header is truncated at {length} bytes")
            }
        }
    }
}

impl std::error::Error for PacketHeaderError {}

pub fn short_packet_checksum(bytes: &[u8]) -> u16 {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut table = [0u32; 256];
        for (index, slot) in table.iter_mut().enumerate() {
            let mut value = index as u32;
            for _ in 0..8 {
                value = (value >> 1) ^ (0xedb8_8320 & (0u32.wrapping_sub(value & 1)));
            }
            *slot = value;
        }
        table
    });
    let mut crc = !0u32;
    for byte in bytes {
        crc = (crc >> 8) ^ table[((crc ^ u32::from(*byte)) & 0xff) as usize];
    }
    let crc = !crc;
    ((crc & 0xffff) ^ (crc >> 16)) as u16
}

pub fn encode_packet_header(
    sequence: i32,
    outgoing_ack: i32,
    reliable_state: u8,
    choked: Option<u8>,
    challenge: Option<u32>,
    checksum_required: bool,
) -> EncodedPacketHeader {
    let mut bytes = [0u8; MAX_ENCODED_PACKET_HEADER_BYTES];
    let mut cursor = 0usize;
    bytes[cursor..cursor + 4].copy_from_slice(&sequence.to_le_bytes());
    cursor += 4;
    bytes[cursor..cursor + 4].copy_from_slice(&outgoing_ack.to_le_bytes());
    cursor += 4;
    let flags_offset = cursor;
    cursor += 1;
    let checksum_offset = checksum_required.then_some(cursor);
    if checksum_required {
        cursor += 2;
    }
    let checksum_start = cursor;
    bytes[cursor] = reliable_state;
    cursor += 1;

    let mut base_flags = 0u8;
    if let Some(choked) = choked {
        base_flags |= PACKET_FLAG_CHOKED;
        bytes[cursor] = choked;
        cursor += 1;
    }
    if let Some(challenge) = challenge {
        base_flags |= PACKET_FLAG_CHALLENGE;
        bytes[cursor..cursor + 4].copy_from_slice(&challenge.to_le_bytes());
        cursor += 4;
    }
    bytes[flags_offset] = base_flags;

    EncodedPacketHeader {
        bytes,
        length: cursor,
        flags_offset,
        checksum_offset,
        checksum_start,
        base_flags,
    }
}

pub fn finalize_packet_header(
    packet: &mut [u8],
    flags: u8,
    checksum_required: bool,
) -> std::result::Result<u16, PacketHeaderError> {
    let minimum = if checksum_required { 11 } else { 9 };
    if packet.len() < minimum {
        return Err(PacketHeaderError::Truncated(packet.len()));
    }
    let checksum = if checksum_required {
        short_packet_checksum(&packet[11..])
    } else {
        0
    };
    packet[8] = flags;
    if checksum_required {
        packet[9..11].copy_from_slice(&checksum.to_le_bytes());
    }
    Ok(checksum)
}

pub fn parse_packet_header(
    packet: &[u8],
    checksum_required: bool,
    expects_challenge: bool,
    expected_challenge: u32,
) -> PacketHeaderDecision {
    fn read<const N: usize>(packet: &[u8], cursor: &mut usize) -> Option<[u8; N]> {
        let end = cursor.checked_add(N)?;
        let bytes = packet.get(*cursor..end)?.try_into().ok()?;
        *cursor = end;
        Some(bytes)
    }

    let mut decision = PacketHeaderDecision {
        reason: PACKET_HEADER_TRUNCATED,
        ..PacketHeaderDecision::default()
    };
    let mut cursor = 0usize;
    let Some(sequence) = read::<4>(packet, &mut cursor) else {
        return decision;
    };
    decision.sequence = i32::from_le_bytes(sequence);
    let Some(outgoing_ack) = read::<4>(packet, &mut cursor) else {
        return decision;
    };
    decision.outgoing_ack = i32::from_le_bytes(outgoing_ack);
    let Some([flags]) = read::<1>(packet, &mut cursor) else {
        return decision;
    };
    decision.flags = flags;

    if checksum_required {
        let Some(checksum) = read::<2>(packet, &mut cursor) else {
            return decision;
        };
        if short_packet_checksum(&packet[cursor..]) != u16::from_le_bytes(checksum) {
            decision.reason = PACKET_HEADER_CHECKSUM_MISMATCH;
            return decision;
        }
    }

    let Some([reliable_state]) = read::<1>(packet, &mut cursor) else {
        return decision;
    };
    decision.reliable_state = reliable_state;
    if flags & PACKET_FLAG_CHOKED != 0 {
        let Some([choked]) = read::<1>(packet, &mut cursor) else {
            return decision;
        };
        decision.choked = choked;
    }

    if flags & PACKET_FLAG_CHALLENGE != 0 {
        let Some(challenge) = read::<4>(packet, &mut cursor) else {
            return decision;
        };
        decision.challenge = u32::from_le_bytes(challenge);
        if decision.challenge != expected_challenge {
            decision.reason = PACKET_HEADER_CHALLENGE_MISMATCH;
            return decision;
        }
    } else if expects_challenge {
        decision.reason = PACKET_HEADER_CHALLENGE_MISSING;
        return decision;
    }

    decision.header_bytes = cursor;
    decision.accepted = true;
    decision.reason = PACKET_HEADER_ACCEPTED;
    decision
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelError {
    Missing(u64),
    SequenceOverflow,
}

impl fmt::Display for ChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(id) => write!(formatter, "network channel {id} is not registered"),
            Self::SequenceOverflow => write!(formatter, "network sequence arithmetic overflow"),
        }
    }
}

impl std::error::Error for ChannelError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChannelState {
    outgoing_sequence: i32,
    incoming_sequence: i32,
    outgoing_ack: i32,
}

#[derive(Debug)]
pub struct ChannelRegistry {
    next_id: u64,
    channels: HashMap<u64, ChannelState>,
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            channels: HashMap::new(),
        }
    }

    pub fn create(
        &mut self,
        outgoing_sequence: i32,
        incoming_sequence: i32,
        outgoing_ack: i32,
    ) -> u64 {
        let id = loop {
            let candidate = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if candidate != 0 && !self.channels.contains_key(&candidate) {
                break candidate;
            }
        };
        self.channels.insert(
            id,
            ChannelState {
                outgoing_sequence,
                incoming_sequence,
                outgoing_ack,
            },
        );
        id
    }

    pub fn remove(&mut self, id: u64) -> bool {
        self.channels.remove(&id).is_some()
    }

    pub fn reset(
        &mut self,
        id: u64,
        outgoing_sequence: i32,
        incoming_sequence: i32,
        outgoing_ack: i32,
    ) -> std::result::Result<(), ChannelError> {
        let state = self
            .channels
            .get_mut(&id)
            .ok_or(ChannelError::Missing(id))?;
        *state = ChannelState {
            outgoing_sequence,
            incoming_sequence,
            outgoing_ack,
        };
        Ok(())
    }

    pub fn advance_outgoing(
        &mut self,
        id: u64,
    ) -> std::result::Result<SequenceAdvance, ChannelError> {
        let state = self
            .channels
            .get_mut(&id)
            .ok_or(ChannelError::Missing(id))?;
        let previous = state.outgoing_sequence;
        state.outgoing_sequence = state.outgoing_sequence.wrapping_add(1);
        Ok(SequenceAdvance {
            previous,
            current: state.outgoing_sequence,
        })
    }

    pub fn preview_incoming(
        &self,
        id: u64,
        sequence: i32,
        outgoing_ack: i32,
        choked: u32,
        max_drop: i32,
    ) -> std::result::Result<PacketDecision, ChannelError> {
        let state = self.channels.get(&id).ok_or(ChannelError::Missing(id))?;
        if sequence <= state.incoming_sequence {
            return Ok(PacketDecision {
                incoming_sequence: state.incoming_sequence,
                outgoing_ack: state.outgoing_ack,
                dropped: 0,
                accepted: false,
                reason: if sequence == state.incoming_sequence {
                    PACKET_DUPLICATE
                } else {
                    PACKET_OUT_OF_ORDER
                },
            });
        }

        let dropped = i64::from(sequence)
            .checked_sub(
                i64::from(state.incoming_sequence)
                    .checked_add(i64::from(choked))
                    .and_then(|value| value.checked_add(1))
                    .ok_or(ChannelError::SequenceOverflow)?,
            )
            .ok_or(ChannelError::SequenceOverflow)?;
        let dropped = i32::try_from(dropped).map_err(|_| ChannelError::SequenceOverflow)?;
        if max_drop > 0 && dropped > max_drop {
            return Ok(PacketDecision {
                incoming_sequence: state.incoming_sequence,
                outgoing_ack: state.outgoing_ack,
                dropped,
                accepted: false,
                reason: PACKET_EXCESSIVE_DROP,
            });
        }

        Ok(PacketDecision {
            incoming_sequence: sequence,
            outgoing_ack,
            dropped,
            accepted: true,
            reason: PACKET_ACCEPTED,
        })
    }

    pub fn commit_incoming(
        &mut self,
        id: u64,
        sequence: i32,
        outgoing_ack: i32,
        choked: u32,
        max_drop: i32,
    ) -> std::result::Result<PacketDecision, ChannelError> {
        let decision = self.preview_incoming(id, sequence, outgoing_ack, choked, max_drop)?;
        if decision.accepted {
            let state = self
                .channels
                .get_mut(&id)
                .ok_or(ChannelError::Missing(id))?;
            state.incoming_sequence = decision.incoming_sequence;
            state.outgoing_ack = decision.outgoing_ack;
        }
        Ok(decision)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringTableError {
    MissingTable(u64),
    MissingEntry(u32),
    MissingString,
    InvalidCapacity(u32),
    InvalidString,
    UserDataTooLarge(usize),
    TableFull(u32),
    TickRegression {
        current: i32,
        requested: i32,
    },
    HistoryDisabled,
    /// The entry count or a user-data length does not fit the 16-bit fields the
    /// wire format uses.
    NotEncodable(usize),
    /// An encoded container ended in the middle of a field.
    Truncated,
}

impl fmt::Display for StringTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingTable(id) => {
                write!(formatter, "network string table {id} is not registered")
            }
            Self::MissingEntry(index) => {
                write!(formatter, "network string entry {index} is missing")
            }
            Self::MissingString => write!(formatter, "network string is not present"),
            Self::InvalidCapacity(capacity) => {
                write!(
                    formatter,
                    "network string table capacity {capacity} is invalid"
                )
            }
            Self::InvalidString => write!(formatter, "network string is invalid"),
            Self::UserDataTooLarge(length) => {
                write!(formatter, "network string user data is too large: {length}")
            }
            Self::TableFull(capacity) => {
                write!(
                    formatter,
                    "network string table is full at {capacity} entries"
                )
            }
            Self::TickRegression { current, requested } => write!(
                formatter,
                "network string table tick regressed from {current} to {requested}"
            ),
            Self::HistoryDisabled => write!(formatter, "network string table history is disabled"),
            Self::NotEncodable(length) => {
                write!(
                    formatter,
                    "network string table length {length} is not encodable"
                )
            }
            Self::Truncated => write!(formatter, "network string table container is truncated"),
        }
    }
}

impl std::error::Error for StringTableError {}

/// Decoded entries as name and user data pairs, in table order.
pub type StringTableEntries = Vec<(Vec<u8>, Vec<u8>)>;

/// One table as it appears inside a demo or save string-table section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringTableSection {
    pub name: Vec<u8>,
    pub entries: StringTableEntries,
    /// Client-side entries, present only when the recording side had them.
    pub client_side: Option<StringTableEntries>,
}

/// Decodes the container a demo `dem_stringtables` command carries: a table
/// count, then each table's name followed by its entries and optional
/// client-side entries.
///
/// This is the inverse of what the engine writes, so a recording can be
/// checked against the encoder that produced it.
pub fn decode_string_table_container(
    bytes: &[u8],
) -> std::result::Result<Vec<StringTableSection>, StringTableError> {
    let mut reader = BitReader::new(bytes);
    let table_count = read_bits(&mut reader, 8)?;
    let mut sections = Vec::with_capacity(table_count as usize);
    for _ in 0..table_count {
        let name = read_cstr(&mut reader)?;
        let entries = read_string_table_entries(&mut reader)?;
        let client_side = if read_bit(&mut reader)? {
            Some(read_string_table_entries(&mut reader)?)
        } else {
            None
        };
        sections.push(StringTableSection {
            name,
            entries,
            client_side,
        });
    }
    Ok(sections)
}

fn read_string_table_entries(
    reader: &mut BitReader<'_>,
) -> std::result::Result<StringTableEntries, StringTableError> {
    let count = read_bits(reader, 16)?;
    let mut entries = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let value = read_cstr(reader)?;
        let user_data = if read_bit(reader)? {
            let length = read_bits(reader, 16)? as usize;
            let mut data = Vec::with_capacity(length);
            for _ in 0..length {
                data.push(read_bits(reader, 8)? as u8);
            }
            data
        } else {
            Vec::new()
        };
        entries.push((value, user_data));
    }
    Ok(entries)
}

fn read_bits(reader: &mut BitReader<'_>, count: u32) -> std::result::Result<u64, StringTableError> {
    reader
        .read_bits(count)
        .map_err(|_| StringTableError::Truncated)
}

fn read_bit(reader: &mut BitReader<'_>) -> std::result::Result<bool, StringTableError> {
    reader.read_bool().map_err(|_| StringTableError::Truncated)
}

fn read_cstr(reader: &mut BitReader<'_>) -> std::result::Result<Vec<u8>, StringTableError> {
    let mut value = Vec::new();
    loop {
        let byte = read_bits(reader, 8)? as u8;
        if byte == 0 {
            return Ok(value);
        }
        if value.len() >= MAX_NETWORK_STRING_BYTES {
            return Err(StringTableError::InvalidString);
        }
        value.push(byte);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StringTableUpsert {
    pub index: u32,
    pub entry_count: u32,
    pub created: bool,
    pub user_data_changed: bool,
    pub tick_changed: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StringTableUserDataChange {
    pub changed: bool,
    pub tick_changed: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StringTableVersion {
    tick: i32,
    user_data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StringTableEntry {
    value: Vec<u8>,
    user_data: Vec<u8>,
    tick_created: i32,
    tick_changed: i32,
    history: Vec<StringTableVersion>,
}

#[derive(Debug)]
struct StringTable {
    name: Vec<u8>,
    max_entries: u32,
    tick: i32,
    last_changed_tick: i32,
    history_enabled: bool,
    entries: Vec<StringTableEntry>,
    lookup: HashMap<Vec<u8>, u32>,
}

impl StringTable {
    fn new(
        name: &[u8],
        max_entries: u32,
        tick: i32,
    ) -> std::result::Result<Self, StringTableError> {
        if max_entries == 0 || !max_entries.is_power_of_two() {
            return Err(StringTableError::InvalidCapacity(max_entries));
        }
        Self::folded(name)?;
        Ok(Self {
            name: name.to_vec(),
            max_entries,
            tick,
            last_changed_tick: 0,
            history_enabled: false,
            entries: Vec::new(),
            lookup: HashMap::new(),
        })
    }

    fn folded(value: &[u8]) -> std::result::Result<Vec<u8>, StringTableError> {
        if value.len() > MAX_NETWORK_STRING_BYTES || value.contains(&0) {
            return Err(StringTableError::InvalidString);
        }
        Ok(value.iter().map(u8::to_ascii_lowercase).collect())
    }

    fn validate_user_data(user_data: &[u8]) -> std::result::Result<(), StringTableError> {
        if user_data.len() > MAX_NETWORK_USER_DATA_BYTES {
            return Err(StringTableError::UserDataTooLarge(user_data.len()));
        }
        Ok(())
    }

    fn record_history(entry: &mut StringTableEntry, tick: i32, user_data: &[u8]) {
        if let Some(last) = entry
            .history
            .last_mut()
            .filter(|version| version.tick == tick)
        {
            last.user_data.clear();
            last.user_data.extend_from_slice(user_data);
        } else if entry
            .history
            .last()
            .is_none_or(|version| version.user_data != user_data)
        {
            entry.history.push(StringTableVersion {
                tick,
                user_data: user_data.to_vec(),
            });
        }
    }

    fn set_user_data_at(
        &mut self,
        index: u32,
        user_data: &[u8],
    ) -> std::result::Result<StringTableUserDataChange, StringTableError> {
        Self::validate_user_data(user_data)?;
        let entry = self
            .entries
            .get_mut(index as usize)
            .ok_or(StringTableError::MissingEntry(index))?;
        if self.history_enabled {
            let already_recorded = entry
                .history
                .last()
                .is_some_and(|version| version.user_data == user_data);
            if !already_recorded {
                Self::record_history(entry, self.tick, user_data);
            }
            return Ok(StringTableUserDataChange {
                changed: false,
                tick_changed: entry.tick_changed,
            });
        }
        if entry.user_data == user_data {
            return Ok(StringTableUserDataChange {
                changed: false,
                tick_changed: entry.tick_changed,
            });
        }
        entry.user_data.clear();
        entry.user_data.extend_from_slice(user_data);
        entry.tick_changed = self.tick;
        self.last_changed_tick = self.tick;
        Ok(StringTableUserDataChange {
            changed: true,
            tick_changed: entry.tick_changed,
        })
    }
}

#[derive(Debug)]
pub struct StringTableRegistry {
    next_id: u64,
    tables: HashMap<u64, StringTable>,
}

impl Default for StringTableRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl StringTableRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            tables: HashMap::new(),
        }
    }

    pub fn create(
        &mut self,
        name: &[u8],
        max_entries: u32,
        tick: i32,
    ) -> std::result::Result<u64, StringTableError> {
        let table = StringTable::new(name, max_entries, tick)?;
        let id = loop {
            let candidate = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if candidate != 0 && !self.tables.contains_key(&candidate) {
                break candidate;
            }
        };
        self.tables.insert(id, table);
        Ok(id)
    }

    pub fn remove(&mut self, id: u64) -> bool {
        self.tables.remove(&id).is_some()
    }

    pub fn clear(&mut self, id: u64) -> std::result::Result<(), StringTableError> {
        let table = self
            .tables
            .get_mut(&id)
            .ok_or(StringTableError::MissingTable(id))?;
        table.entries.clear();
        table.lookup.clear();
        table.last_changed_tick = 0;
        Ok(())
    }

    pub fn enable_history(&mut self, id: u64) -> std::result::Result<(), StringTableError> {
        let table = self
            .tables
            .get_mut(&id)
            .ok_or(StringTableError::MissingTable(id))?;
        table.history_enabled = true;
        for entry in &mut table.entries {
            if entry.history.is_empty() {
                entry.history.push(StringTableVersion {
                    tick: entry.tick_created,
                    user_data: entry.user_data.clone(),
                });
            }
        }
        Ok(())
    }

    pub fn set_tick(&mut self, id: u64, tick: i32) -> std::result::Result<(), StringTableError> {
        let table = self
            .tables
            .get_mut(&id)
            .ok_or(StringTableError::MissingTable(id))?;
        if tick < table.tick {
            return Err(StringTableError::TickRegression {
                current: table.tick,
                requested: tick,
            });
        }
        table.tick = tick;
        Ok(())
    }

    pub fn synchronize_tick(
        &mut self,
        id: u64,
        tick: i32,
    ) -> std::result::Result<(), StringTableError> {
        let table = self
            .tables
            .get_mut(&id)
            .ok_or(StringTableError::MissingTable(id))?;
        table.tick = tick;
        Ok(())
    }

    pub fn upsert(
        &mut self,
        id: u64,
        value: &[u8],
        user_data: Option<&[u8]>,
    ) -> std::result::Result<StringTableUpsert, StringTableError> {
        let folded = StringTable::folded(value)?;
        if let Some(user_data) = user_data {
            StringTable::validate_user_data(user_data)?;
        }
        let table = self
            .tables
            .get_mut(&id)
            .ok_or(StringTableError::MissingTable(id))?;
        if let Some(index) = table.lookup.get(&folded).copied() {
            let change = match user_data {
                Some(user_data) => table.set_user_data_at(index, user_data)?,
                None => StringTableUserDataChange {
                    changed: false,
                    tick_changed: table.entries[index as usize].tick_changed,
                },
            };
            return Ok(StringTableUpsert {
                index,
                entry_count: table.entries.len() as u32,
                created: false,
                user_data_changed: change.changed,
                tick_changed: change.tick_changed,
            });
        }
        if table.entries.len() >= table.max_entries as usize {
            return Err(StringTableError::TableFull(table.max_entries));
        }
        let index = table.entries.len() as u32;
        let user_data = user_data.unwrap_or_default().to_vec();
        let mut entry = StringTableEntry {
            value: value.to_vec(),
            user_data: if table.history_enabled {
                Vec::new()
            } else {
                user_data.clone()
            },
            tick_created: table.tick,
            tick_changed: table.tick,
            history: Vec::new(),
        };
        if table.history_enabled {
            entry.history.push(StringTableVersion {
                tick: table.tick,
                user_data,
            });
        }
        table.entries.push(entry);
        table.lookup.insert(folded, index);
        if !table.history_enabled {
            table.last_changed_tick = table.tick;
        }
        Ok(StringTableUpsert {
            index,
            entry_count: table.entries.len() as u32,
            created: true,
            user_data_changed: false,
            tick_changed: table.tick,
        })
    }

    pub fn set_user_data(
        &mut self,
        id: u64,
        index: u32,
        user_data: &[u8],
    ) -> std::result::Result<StringTableUserDataChange, StringTableError> {
        self.tables
            .get_mut(&id)
            .ok_or(StringTableError::MissingTable(id))?
            .set_user_data_at(index, user_data)
    }

    pub fn find(&self, id: u64, value: &[u8]) -> std::result::Result<u32, StringTableError> {
        let folded = StringTable::folded(value)?;
        self.tables
            .get(&id)
            .ok_or(StringTableError::MissingTable(id))?
            .lookup
            .get(&folded)
            .copied()
            .ok_or(StringTableError::MissingString)
    }

    pub fn changed_since(&self, id: u64, tick: i32) -> std::result::Result<bool, StringTableError> {
        Ok(self
            .tables
            .get(&id)
            .ok_or(StringTableError::MissingTable(id))?
            .last_changed_tick
            > tick)
    }

    pub fn restore_tick(
        &mut self,
        id: u64,
        tick: i32,
    ) -> std::result::Result<i32, StringTableError> {
        let table = self
            .tables
            .get_mut(&id)
            .ok_or(StringTableError::MissingTable(id))?;
        if !table.history_enabled {
            return Err(StringTableError::HistoryDisabled);
        }
        let mut last_changed_tick = 0;
        for entry in &mut table.entries {
            let version = entry
                .history
                .iter()
                .rev()
                .find(|version| version.tick <= tick);
            if let Some(version) = version {
                entry.user_data.clone_from(&version.user_data);
                entry.tick_changed = version.tick;
                last_changed_tick = last_changed_tick.max(version.tick);
            } else {
                entry.user_data.clear();
                entry.tick_changed = 0;
            }
        }
        table.last_changed_tick = last_changed_tick;
        Ok(last_changed_tick)
    }

    pub fn entry_count(&self, id: u64) -> std::result::Result<u32, StringTableError> {
        Ok(self
            .tables
            .get(&id)
            .ok_or(StringTableError::MissingTable(id))?
            .entries
            .len() as u32)
    }

    pub fn name(&self, id: u64) -> std::result::Result<&[u8], StringTableError> {
        Ok(&self
            .tables
            .get(&id)
            .ok_or(StringTableError::MissingTable(id))?
            .name)
    }

    pub fn string(&self, id: u64, index: u32) -> std::result::Result<&[u8], StringTableError> {
        Ok(&self
            .tables
            .get(&id)
            .ok_or(StringTableError::MissingTable(id))?
            .entries
            .get(index as usize)
            .ok_or(StringTableError::MissingEntry(index))?
            .value)
    }

    pub fn user_data(&self, id: u64, index: u32) -> std::result::Result<&[u8], StringTableError> {
        Ok(&self
            .tables
            .get(&id)
            .ok_or(StringTableError::MissingTable(id))?
            .entries
            .get(index as usize)
            .ok_or(StringTableError::MissingEntry(index))?
            .user_data)
    }

    /// Encodes the table's own entries in the layout demo and save containers
    /// use: a 16-bit count, then per entry a NUL-terminated name, a flag bit,
    /// and, when the flag is set, a 16-bit length followed by that many user
    /// data bytes.
    ///
    /// Client-side entries are not part of this, because they are not table
    /// state; the caller appends that section after these bits.
    pub fn encode_entries(&self, id: u64) -> std::result::Result<(Vec<u8>, u32), StringTableError> {
        let table = self
            .tables
            .get(&id)
            .ok_or(StringTableError::MissingTable(id))?;
        let count = u16::try_from(table.entries.len())
            .map_err(|_| StringTableError::NotEncodable(table.entries.len()))?;

        let mut writer = BitWriter::new();
        let encode = |writer: &mut BitWriter, table: &StringTable| -> Option<()> {
            writer.write_bits(u64::from(count), 16).ok()?;
            for entry in &table.entries {
                for byte in &entry.value {
                    writer.write_bits(u64::from(*byte), 8).ok()?;
                }
                writer.write_bits(0, 8).ok()?;
                if entry.user_data.is_empty() {
                    writer.write_bool(false).ok()?;
                    continue;
                }
                let length = u16::try_from(entry.user_data.len()).ok()?;
                writer.write_bool(true).ok()?;
                writer.write_bits(u64::from(length), 16).ok()?;
                for byte in &entry.user_data {
                    writer.write_bits(u64::from(*byte), 8).ok()?;
                }
            }
            Some(())
        };
        if encode(&mut writer, table).is_none() {
            let longest = table
                .entries
                .iter()
                .map(|entry| entry.user_data.len())
                .max()
                .unwrap_or(0);
            return Err(StringTableError::NotEncodable(longest));
        }

        let bits = u32::try_from(writer.len_bits())
            .map_err(|_| StringTableError::NotEncodable(writer.len_bits()))?;
        Ok((writer.as_slice().to_vec(), bits))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataTablePropertyDescriptor {
    pub name: Vec<u8>,
    pub reference_name: Vec<u8>,
    pub property_type: u32,
    pub flags: u32,
    pub bit_count: i32,
    pub elements: u32,
    pub low_value: f32,
    pub high_value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataTableRegistration {
    pub table_id: u32,
    pub created: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataTableSummary {
    pub table_count: u32,
    pub property_count: u32,
    pub class_count: u32,
    pub compatibility_crc: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataTableError {
    Finalized,
    NotFinalized,
    InvalidName,
    TooManyTables,
    TooManyProperties(u32),
    TooManyClasses,
    ConflictingTable(Vec<u8>),
    MissingTable(u32),
    MissingClass,
    DuplicateClass(Vec<u8>),
    InvalidPropertyType(u32),
    InvalidPropertyFlags(u32),
    InvalidBitCount(i32),
    InvalidElementCount(u32),
    UnexpectedReference,
    MissingReference,
    PropertyCountMismatch {
        table_id: u32,
        expected: u32,
        actual: u32,
    },
    UnknownReferencedTable(Vec<u8>),
    EmptyClasses,
    CompatibilityCrcMismatch {
        expected: u32,
        actual: u32,
    },
}

impl fmt::Display for DataTableError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Finalized => write!(formatter, "datatable schema is already finalized"),
            Self::NotFinalized => write!(formatter, "datatable schema is not finalized"),
            Self::InvalidName => write!(formatter, "datatable name is invalid"),
            Self::TooManyTables => write!(formatter, "datatable schema exceeds its table limit"),
            Self::TooManyProperties(count) => {
                write!(formatter, "datatable declares too many properties: {count}")
            }
            Self::TooManyClasses => write!(formatter, "datatable schema exceeds its class limit"),
            Self::ConflictingTable(name) => write!(
                formatter,
                "datatable has a conflicting declaration: {}",
                String::from_utf8_lossy(name)
            ),
            Self::MissingTable(id) => write!(formatter, "datatable {id} is not registered"),
            Self::MissingClass => write!(formatter, "server class is not registered"),
            Self::DuplicateClass(name) => write!(
                formatter,
                "server class is duplicated: {}",
                String::from_utf8_lossy(name)
            ),
            Self::InvalidPropertyType(property_type) => {
                write!(formatter, "send property type {property_type} is invalid")
            }
            Self::InvalidPropertyFlags(flags) => {
                write!(formatter, "send property flags {flags:#x} are invalid")
            }
            Self::InvalidBitCount(bits) => {
                write!(formatter, "send property bit count {bits} is invalid")
            }
            Self::InvalidElementCount(elements) => {
                write!(
                    formatter,
                    "send property element count {elements} is invalid"
                )
            }
            Self::UnexpectedReference => {
                write!(formatter, "send property has an unexpected table reference")
            }
            Self::MissingReference => {
                write!(formatter, "send property is missing its table reference")
            }
            Self::PropertyCountMismatch {
                table_id,
                expected,
                actual,
            } => write!(
                formatter,
                "datatable {table_id} expected {expected} properties but received {actual}"
            ),
            Self::UnknownReferencedTable(name) => write!(
                formatter,
                "send property references an unknown table: {}",
                String::from_utf8_lossy(name)
            ),
            Self::EmptyClasses => write!(formatter, "datatable schema has no server classes"),
            Self::CompatibilityCrcMismatch { expected, actual } => write!(
                formatter,
                "datatable compatibility CRC mismatch: expected {expected:08x}, got {actual:08x}"
            ),
        }
    }
}

impl std::error::Error for DataTableError {}

#[derive(Debug, Clone, PartialEq)]
struct DataTableDefinition {
    name: Vec<u8>,
    expected_properties: u32,
    properties: Vec<DataTablePropertyDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ServerClassDefinition {
    name: Vec<u8>,
    table_id: u32,
}

#[derive(Debug, Default)]
pub struct DataTableRegistry {
    tables: Vec<DataTableDefinition>,
    table_lookup: HashMap<Vec<u8>, u32>,
    classes: Vec<ServerClassDefinition>,
    class_lookup: HashMap<Vec<u8>, u32>,
    finalized: bool,
    compatibility_crc: u32,
}

impl DataTableRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn validate_name(name: &[u8]) -> std::result::Result<(), DataTableError> {
        if name.is_empty()
            || name.len() > MAX_DATA_TABLE_NAME_BYTES
            || name.contains(&0)
            || !name.is_ascii()
        {
            return Err(DataTableError::InvalidName);
        }
        Ok(())
    }

    fn folded(name: &[u8]) -> Vec<u8> {
        name.iter().map(u8::to_ascii_lowercase).collect()
    }

    pub fn register_table(
        &mut self,
        name: &[u8],
        expected_properties: u32,
    ) -> std::result::Result<DataTableRegistration, DataTableError> {
        if self.finalized {
            return Err(DataTableError::Finalized);
        }
        Self::validate_name(name)?;
        if expected_properties as usize > MAX_DATA_TABLE_PROPERTIES {
            return Err(DataTableError::TooManyProperties(expected_properties));
        }
        if let Some(table_id) = self.table_lookup.get(name).copied() {
            let table = &self.tables[table_id as usize];
            if table.expected_properties != expected_properties {
                return Err(DataTableError::ConflictingTable(name.to_vec()));
            }
            return Ok(DataTableRegistration {
                table_id,
                created: false,
            });
        }
        if self.tables.len() >= MAX_DATA_TABLES {
            return Err(DataTableError::TooManyTables);
        }
        let table_id = self.tables.len() as u32;
        self.tables.push(DataTableDefinition {
            name: name.to_vec(),
            expected_properties,
            properties: Vec::with_capacity(expected_properties as usize),
        });
        self.table_lookup.insert(name.to_vec(), table_id);
        Ok(DataTableRegistration {
            table_id,
            created: true,
        })
    }

    pub fn register_property(
        &mut self,
        table_id: u32,
        property: DataTablePropertyDescriptor,
    ) -> std::result::Result<(), DataTableError> {
        if self.finalized {
            return Err(DataTableError::Finalized);
        }
        Self::validate_name(&property.name)?;
        if property.property_type > SEND_PROP_DATA_TABLE {
            return Err(DataTableError::InvalidPropertyType(property.property_type));
        }
        if property.flags & !SEND_PROP_FLAG_MASK != 0 {
            return Err(DataTableError::InvalidPropertyFlags(property.flags));
        }
        let requires_reference = property.property_type == SEND_PROP_DATA_TABLE
            || property.flags & SEND_PROP_EXCLUDE != 0;
        if requires_reference {
            Self::validate_name(&property.reference_name)
                .map_err(|_| DataTableError::MissingReference)?;
        } else if !property.reference_name.is_empty() {
            return Err(DataTableError::UnexpectedReference);
        }
        if property.property_type == SEND_PROP_ARRAY {
            if property.elements == 0 || property.elements >= (1 << 10) {
                return Err(DataTableError::InvalidElementCount(property.elements));
            }
        } else if property.elements != 1 {
            return Err(DataTableError::InvalidElementCount(property.elements));
        }
        if property.property_type != SEND_PROP_DATA_TABLE
            && property.property_type != SEND_PROP_ARRAY
            && property.flags & SEND_PROP_EXCLUDE == 0
            && !(-2..=127).contains(&property.bit_count)
        {
            return Err(DataTableError::InvalidBitCount(property.bit_count));
        }
        let table = self
            .tables
            .get_mut(table_id as usize)
            .ok_or(DataTableError::MissingTable(table_id))?;
        if table.properties.len() >= table.expected_properties as usize {
            return Err(DataTableError::TooManyProperties(
                table.expected_properties.saturating_add(1),
            ));
        }
        table.properties.push(property);
        Ok(())
    }

    pub fn register_class(
        &mut self,
        name: &[u8],
        table_id: u32,
    ) -> std::result::Result<u32, DataTableError> {
        if self.finalized {
            return Err(DataTableError::Finalized);
        }
        Self::validate_name(name)?;
        if table_id as usize >= self.tables.len() {
            return Err(DataTableError::MissingTable(table_id));
        }
        if self.classes.len() >= MAX_SERVER_CLASSES {
            return Err(DataTableError::TooManyClasses);
        }
        let folded = Self::folded(name);
        if self.class_lookup.contains_key(&folded) {
            return Err(DataTableError::DuplicateClass(name.to_vec()));
        }
        let class_id = self.classes.len() as u32;
        self.classes.push(ServerClassDefinition {
            name: name.to_vec(),
            table_id,
        });
        self.class_lookup.insert(folded, class_id);
        Ok(class_id)
    }

    fn update_crc(mut crc: u32, bytes: &[u8]) -> u32 {
        for byte in bytes {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        crc
    }

    fn compute_compatibility_crc(&self) -> u32 {
        let mut crc = u32::MAX;
        for class in &self.classes {
            let table = &self.tables[class.table_id as usize];
            crc = Self::update_crc(crc, &table.name);
            crc = Self::update_crc(crc, &(table.properties.len() as i32).to_le_bytes());
            for property in &table.properties {
                crc = Self::update_crc(crc, &(property.property_type as i32).to_le_bytes());
                crc = Self::update_crc(crc, &property.name);
                crc = Self::update_crc(crc, &(property.flags as i32).to_le_bytes());
                if property.property_type == SEND_PROP_DATA_TABLE
                    || property.flags & SEND_PROP_EXCLUDE != 0
                {
                    crc = Self::update_crc(crc, &property.reference_name);
                } else if property.property_type == SEND_PROP_ARRAY {
                    crc = Self::update_crc(crc, &(property.elements as i32).to_le_bytes());
                } else {
                    crc = Self::update_crc(crc, &property.low_value.to_bits().to_le_bytes());
                    crc = Self::update_crc(crc, &property.high_value.to_bits().to_le_bytes());
                    crc = Self::update_crc(crc, &property.bit_count.to_le_bytes());
                }
            }
        }
        !crc
    }

    pub fn finalize(
        &mut self,
        expected_compatibility_crc: u32,
    ) -> std::result::Result<DataTableSummary, DataTableError> {
        if self.finalized {
            return Err(DataTableError::Finalized);
        }
        if self.classes.is_empty() {
            return Err(DataTableError::EmptyClasses);
        }
        for (table_id, table) in self.tables.iter().enumerate() {
            if table.properties.len() != table.expected_properties as usize {
                return Err(DataTableError::PropertyCountMismatch {
                    table_id: table_id as u32,
                    expected: table.expected_properties,
                    actual: table.properties.len() as u32,
                });
            }
            for property in &table.properties {
                if (property.property_type == SEND_PROP_DATA_TABLE
                    || property.flags & SEND_PROP_EXCLUDE != 0)
                    && !self.table_lookup.contains_key(&property.reference_name)
                {
                    return Err(DataTableError::UnknownReferencedTable(
                        property.reference_name.clone(),
                    ));
                }
            }
        }
        let compatibility_crc = self.compute_compatibility_crc();
        if compatibility_crc != expected_compatibility_crc {
            return Err(DataTableError::CompatibilityCrcMismatch {
                expected: expected_compatibility_crc,
                actual: compatibility_crc,
            });
        }
        self.compatibility_crc = compatibility_crc;
        self.finalized = true;
        Ok(self.summary_unchecked())
    }

    fn summary_unchecked(&self) -> DataTableSummary {
        DataTableSummary {
            table_count: self.tables.len() as u32,
            property_count: self
                .tables
                .iter()
                .map(|table| table.properties.len() as u32)
                .sum(),
            class_count: self.classes.len() as u32,
            compatibility_crc: self.compatibility_crc,
        }
    }

    pub fn summary(&self) -> std::result::Result<DataTableSummary, DataTableError> {
        if !self.finalized {
            return Err(DataTableError::NotFinalized);
        }
        Ok(self.summary_unchecked())
    }

    pub fn class_id(&self, name: &[u8]) -> std::result::Result<u32, DataTableError> {
        if !self.finalized {
            return Err(DataTableError::NotFinalized);
        }
        Self::validate_name(name)?;
        self.class_lookup
            .get(&Self::folded(name))
            .copied()
            .ok_or(DataTableError::MissingClass)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotEntity {
    pub entity_index: u32,
    pub serial_number: i32,
    pub class_id: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SnapshotSummary {
    pub tick: i32,
    pub max_entities: u32,
    pub valid_entity_count: u32,
    pub explicit_delete_count: u32,
}

/// How one entity index changes between two canonical snapshots.
///
/// The classification is decided entirely by snapshot metadata, so it does not
/// depend on packed entity payloads. Distinguishing an unchanged entity from a
/// changed one requires comparing those payloads and stays with the caller,
/// which is why both collapse into [`SnapshotDeltaKind::DeltaCandidate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotDeltaKind {
    /// Absent from the older snapshot, or present with a different identity, so
    /// it has to be created from its baseline rather than delta-compressed.
    EnterPvs,
    /// Present in the older snapshot and absent from the newer one.
    LeavePvs,
    /// Present in both under the same identity, so it may be delta-compressed.
    DeltaCandidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotDeltaEntry {
    pub entity_index: u32,
    pub kind: SnapshotDeltaKind,
    /// Identity in the newer snapshot, or in the older one for `LeavePvs`.
    pub class_id: u32,
    pub serial_number: i32,
    /// Set when the entity kept its index but changed identity, which the
    /// legacy writer treats as a recreate that consumes both cursors.
    pub recreated: bool,
}

/// The per-entity header that precedes each entity update in a delta packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeltaHeader {
    pub entity_index: u32,
    /// Index of the previously written entity, or -1 before the first one. The
    /// header carries only the gap since that entity.
    pub header_base: i32,
    pub leave_pvs: bool,
    /// Only meaningful together with `leave_pvs`; a removal that also frees the
    /// entity slot.
    pub delete_entity: bool,
    /// Only meaningful without `leave_pvs`; a creation rather than a delta.
    pub enter_pvs: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeltaHeaderError {
    /// Entity indices have to ascend, so the gap can never run backwards.
    NonAscendingIndex {
        header_base: i32,
        entity_index: u32,
    },
    InvalidEntityIndex(u32),
    /// A removal cannot also be a creation.
    ConflictingFlags,
}

impl fmt::Display for DeltaHeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonAscendingIndex {
                header_base,
                entity_index,
            } => write!(
                formatter,
                "delta header entity {entity_index} does not follow header base {header_base}"
            ),
            Self::InvalidEntityIndex(index) => {
                write!(formatter, "delta header entity index {index} is invalid")
            }
            Self::ConflictingFlags => {
                write!(formatter, "delta header cannot both remove and create")
            }
        }
    }
}

impl std::error::Error for DeltaHeaderError {}

/// One piece of a datagram too large to route whole.
///
/// The wire layout is the packed `SPLITPACKET` header: the split flag, the
/// sequence shared by every piece of one message, then two sixteen-bit fields
/// holding the piece's position and the size the sender cut to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitPacketHeader {
    /// Shared by every piece of one message, so a receiver can tell a new
    /// message from a straggler belonging to the last one.
    pub sequence: i32,
    /// Which piece this is, counting from zero.
    pub packet_number: u8,
    /// How many pieces the whole message was cut into.
    pub packet_count: u8,
    /// The payload size the sender cut to, excluding this header. Every piece
    /// but the last carries exactly this much.
    pub split_size: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitPacketError {
    /// The datagram is too short to hold a header.
    Truncated(usize),
    /// The leading word is not the split flag.
    NotSplit(i32),
    /// The sender's split size is outside what any routable datagram allows.
    SplitSizeOutOfRange(u16),
    /// A piece claims a position at or beyond the count it declares.
    PositionOutOfRange { number: u8, count: u8 },
    /// More pieces were declared than the smallest split size could need.
    TooManyPieces(usize),
    /// A later piece disagrees with the first about the split size, so the
    /// offsets computed from it would not line up.
    InconsistentSplitSize { expected: u16, found: u16 },
    /// A piece other than the last is not exactly the declared split size.
    ShortPiece {
        number: u8,
        expected: usize,
        found: usize,
    },
    /// The reassembled message would exceed what the protocol can carry.
    TooLarge(usize),
    /// The payload cannot be cut into at most 255 pieces at this size.
    Unsplittable { payload: usize, split_size: usize },
    /// There is nothing to split. A datagram is only ever cut up because it
    /// was too large to route whole, so an empty one never reaches here.
    EmptyPayload,
}

impl fmt::Display for SplitPacketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated(length) => write!(
                formatter,
                "a split packet needs {SPLIT_PACKET_HEADER_BYTES} header bytes but has {length}"
            ),
            Self::NotSplit(flag) => {
                write!(formatter, "leading word {flag} is not the split flag")
            }
            Self::SplitSizeOutOfRange(size) => write!(
                formatter,
                "split size {size} is outside [{MIN_SPLIT_PAYLOAD_BYTES}, {MAX_SPLIT_PAYLOAD_BYTES}]"
            ),
            Self::PositionOutOfRange { number, count } => {
                write!(formatter, "piece {number} is not within a count of {count}")
            }
            Self::TooManyPieces(count) => {
                write!(formatter, "{count} pieces exceeds the {MAX_SPLIT_COUNT} maximum")
            }
            Self::InconsistentSplitSize { expected, found } => write!(
                formatter,
                "split size {found} does not match the {expected} this message started with"
            ),
            Self::ShortPiece {
                number,
                expected,
                found,
            } => write!(
                formatter,
                "piece {number} carries {found} bytes where only the last may be under {expected}"
            ),
            Self::TooLarge(size) => write!(
                formatter,
                "a reassembled {size} bytes exceeds the {MAX_REASSEMBLED_BYTES} maximum"
            ),
            Self::Unsplittable {
                payload,
                split_size,
            } => write!(
                formatter,
                "{payload} bytes cannot be cut into at most 255 pieces of {split_size}"
            ),
            Self::EmptyPayload => write!(formatter, "there is nothing to split"),
        }
    }
}

impl std::error::Error for SplitPacketError {}

impl SplitPacketHeader {
    /// Encodes the header exactly as the packed C struct lays it out.
    pub fn encode(&self) -> [u8; SPLIT_PACKET_HEADER_BYTES] {
        let mut bytes = [0u8; SPLIT_PACKET_HEADER_BYTES];
        bytes[0..4].copy_from_slice(&SPLIT_PACKET_FLAG.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.sequence.to_le_bytes());
        // The two sixteen-bit fields share one word, position in the high
        // byte and count in the low one.
        let packet_id = (u16::from(self.packet_number) << 8) | u16::from(self.packet_count);
        bytes[8..10].copy_from_slice(&packet_id.to_le_bytes());
        bytes[10..12].copy_from_slice(&self.split_size.to_le_bytes());
        bytes
    }

    /// Reads the header off the front of a datagram.
    ///
    /// The position is read as an unsigned byte. The native reader sign
    /// extends it through a signed `short`, which for a message in more than
    /// 128 pieces yields a negative index it then uses unchecked; refusing to
    /// reproduce that is the one deliberate difference here, and it does not
    /// change how any well-formed piece decodes.
    pub fn decode(bytes: &[u8]) -> std::result::Result<Self, SplitPacketError> {
        if bytes.len() < SPLIT_PACKET_HEADER_BYTES {
            return Err(SplitPacketError::Truncated(bytes.len()));
        }
        let flag = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if flag != SPLIT_PACKET_FLAG {
            return Err(SplitPacketError::NotSplit(flag));
        }

        let sequence = i32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let packet_id = u16::from_le_bytes([bytes[8], bytes[9]]);
        let split_size = u16::from_le_bytes([bytes[10], bytes[11]]);

        let header = Self {
            sequence,
            packet_number: (packet_id >> 8) as u8,
            packet_count: (packet_id & 0xff) as u8,
            split_size,
        };

        let size = usize::from(header.split_size);
        if !(MIN_SPLIT_PAYLOAD_BYTES..=MAX_SPLIT_PAYLOAD_BYTES).contains(&size) {
            return Err(SplitPacketError::SplitSizeOutOfRange(header.split_size));
        }
        if header.packet_count == 0 || header.packet_number >= header.packet_count {
            return Err(SplitPacketError::PositionOutOfRange {
                number: header.packet_number,
                count: header.packet_count,
            });
        }
        Ok(header)
    }
}

/// Cuts a payload into routable pieces, each carrying its own header.
///
/// `max_routable` is the whole datagram size the sender is willing to put on
/// the wire, so the payload each piece carries is that less the header.
pub fn split_packet(
    payload: &[u8],
    sequence: i32,
    max_routable: usize,
) -> std::result::Result<Vec<Vec<u8>>, SplitPacketError> {
    let split_size = max_routable.saturating_sub(SPLIT_PACKET_HEADER_BYTES);
    if !(MIN_SPLIT_PAYLOAD_BYTES..=MAX_SPLIT_PAYLOAD_BYTES).contains(&split_size) {
        return Err(SplitPacketError::SplitSizeOutOfRange(
            u16::try_from(split_size).unwrap_or(u16::MAX),
        ));
    }
    if payload.is_empty() {
        return Err(SplitPacketError::EmptyPayload);
    }
    if payload.len() > MAX_REASSEMBLED_BYTES {
        return Err(SplitPacketError::TooLarge(payload.len()));
    }

    // The count travels in one byte, so a payload needing more pieces than
    // that cannot be described on the wire at this size.
    let count = payload.len().div_ceil(split_size);
    if count > 255 {
        return Err(SplitPacketError::Unsplittable {
            payload: payload.len(),
            split_size,
        });
    }

    let mut pieces = Vec::with_capacity(count);
    for (number, chunk) in payload.chunks(split_size).enumerate() {
        let header = SplitPacketHeader {
            sequence,
            packet_number: number as u8,
            packet_count: count as u8,
            split_size: split_size as u16,
        };
        let mut piece = Vec::with_capacity(SPLIT_PACKET_HEADER_BYTES + chunk.len());
        piece.extend_from_slice(&header.encode());
        piece.extend_from_slice(chunk);
        pieces.push(piece);
    }
    Ok(pieces)
}

/// Rebuilds messages from the pieces one peer sends.
///
/// One of these belongs to one remote address, which is how the native
/// receiver keys its own table; pieces from different peers must not be fed
/// to the same reassembler or their sequences would collide.
#[derive(Debug, Default)]
pub struct SplitPacketReassembler {
    sequence: Option<i32>,
    split_size: u16,
    expected_count: u8,
    /// Which pieces have arrived, so a duplicate is not counted twice.
    seen: Vec<bool>,
    buffer: Vec<u8>,
    total: usize,
    outstanding: usize,
}

impl SplitPacketReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether a message is part-way through being rebuilt.
    pub fn is_assembling(&self) -> bool {
        self.sequence.is_some()
    }

    /// Forgets any partial message, for a peer that has gone quiet long
    /// enough that its pieces are stale.
    pub fn reset(&mut self) {
        self.sequence = None;
        self.seen.clear();
        self.buffer.clear();
        self.total = 0;
        self.outstanding = 0;
    }

    /// Takes one datagram, returning the whole message once its last piece
    /// arrives.
    pub fn accept(
        &mut self,
        datagram: &[u8],
    ) -> std::result::Result<Option<Vec<u8>>, SplitPacketError> {
        let header = SplitPacketHeader::decode(datagram)?;
        let payload = &datagram[SPLIT_PACKET_HEADER_BYTES..];
        let split_size = usize::from(header.split_size);
        let count = usize::from(header.packet_count);

        if count > MAX_SPLIT_COUNT {
            return Err(SplitPacketError::TooManyPieces(count));
        }
        // Only the final piece may be short. Accepting a short middle piece
        // would leave the gap between it and the next offset holding whatever
        // the buffer already had.
        let is_last = usize::from(header.packet_number) + 1 == count;
        if (!is_last && payload.len() != split_size) || payload.len() > split_size {
            return Err(SplitPacketError::ShortPiece {
                number: header.packet_number,
                expected: split_size,
                found: payload.len(),
            });
        }

        // A different sequence means the previous message was abandoned
        // part-way; its pieces will never complete, so it is dropped.
        if self.sequence != Some(header.sequence) {
            self.begin_internal(&header, count);
        } else if self.split_size != header.split_size {
            // Offsets are computed from the split size, so a piece that
            // disagrees cannot be placed. The native receiver also holds the
            // peer off until its partial message goes stale.
            let expected = self.split_size;
            self.reset();
            return Err(SplitPacketError::InconsistentSplitSize {
                expected,
                found: header.split_size,
            });
        }

        let number = usize::from(header.packet_number);
        if number >= self.expected_count as usize {
            return Err(SplitPacketError::PositionOutOfRange {
                number: header.packet_number,
                count: self.expected_count,
            });
        }

        if is_last {
            self.total = (count - 1) * split_size + payload.len();
            if self.total > MAX_REASSEMBLED_BYTES {
                self.reset();
                return Err(SplitPacketError::TooLarge(self.total));
            }
        }

        let offset = number * split_size;
        let end = offset + payload.len();
        if end > self.buffer.len() {
            self.buffer.resize(end, 0);
        }
        self.buffer[offset..end].copy_from_slice(payload);

        // A duplicate overwrites the same bytes but must not be counted, or
        // the message would be reported complete before it is.
        if !self.seen[number] {
            self.seen[number] = true;
            self.outstanding -= 1;
        }

        if self.outstanding > 0 {
            return Ok(None);
        }

        let mut message = std::mem::take(&mut self.buffer);
        message.truncate(self.total);
        self.reset();
        Ok(Some(message))
    }

    fn begin_internal(&mut self, header: &SplitPacketHeader, count: usize) {
        self.sequence = Some(header.sequence);
        self.split_size = header.split_size;
        self.expected_count = header.packet_count;
        self.seen.clear();
        self.seen.resize(count, false);
        self.buffer.clear();
        self.total = 0;
        self.outstanding = count;
    }
}

/// One reassembler per peer, since the native receiver keys its table by the
/// address a datagram arrived from and sequences only mean anything within
/// one sender.
#[derive(Debug, Default)]
pub struct SplitPacketRegistry {
    next_id: u64,
    peers: HashMap<u64, SplitPacketReassembler>,
    /// Messages finished but not yet taken, so a caller whose buffer was too
    /// small can size it and come back rather than lose the message.
    completed: HashMap<u64, Vec<u8>>,
}

impl SplitPacketRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts tracking a peer, returning the id its datagrams arrive under.
    pub fn create(&mut self) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.peers.insert(id, SplitPacketReassembler::new());
        id
    }

    pub fn remove(&mut self, id: u64) -> bool {
        self.completed.remove(&id);
        self.peers.remove(&id).is_some()
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut SplitPacketReassembler> {
        self.peers.get_mut(&id)
    }

    /// Holds a finished message for a peer until it is taken.
    pub fn hold(&mut self, id: u64, message: Vec<u8>) {
        self.completed.insert(id, message);
    }

    /// How long the message waiting for a peer is, if there is one.
    pub fn held_len(&self, id: u64) -> Option<usize> {
        self.completed.get(&id).map(Vec::len)
    }

    pub fn take_held(&mut self, id: u64) -> Option<Vec<u8>> {
        self.completed.remove(&id)
    }

    pub fn len(&self) -> usize {
        self.peers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }

    /// Drops every peer, for a socket that has been closed.
    pub fn clear(&mut self) {
        self.peers.clear();
        self.completed.clear();
    }
}

/// The widest encoding is a 34-bit index gap plus the two flag bits.
pub const MAX_DELTA_HEADER_BITS: u32 = 36;

impl DeltaHeader {
    /// Encodes the header into protocol-25 packet-entity bits.
    ///
    /// The result is packed least-significant-bit first within each byte and
    /// low byte first, which is the layout the wire format uses.
    pub fn encode(&self) -> std::result::Result<(Vec<u8>, u32), DeltaHeaderError> {
        if self.entity_index as usize >= MAX_SNAPSHOT_ENTITIES {
            return Err(DeltaHeaderError::InvalidEntityIndex(self.entity_index));
        }
        if self.leave_pvs && self.enter_pvs {
            return Err(DeltaHeaderError::ConflictingFlags);
        }
        let gap = i64::from(self.entity_index) - i64::from(self.header_base) - 1;
        if gap < 0 {
            return Err(DeltaHeaderError::NonAscendingIndex {
                header_base: self.header_base,
                entity_index: self.entity_index,
            });
        }

        let mut writer = BitWriter::new();
        // Unwraps are safe because every write below is well under 64 bits.
        let gap = gap as u64;
        let (selector, payload_bits) = match gap {
            0x0..=0xf => (0u64, 4u32),
            0x10..=0xff => (1, 8),
            0x100..=0xfff => (2, 12),
            _ => (3, 32),
        };
        writer.write_bits(selector, 2).expect("fits");
        writer.write_bits(gap, payload_bits).expect("fits");

        if self.leave_pvs {
            writer.write_bool(true).expect("fits");
            writer.write_bool(self.delete_entity).expect("fits");
        } else {
            writer.write_bool(false).expect("fits");
            writer.write_bool(self.enter_pvs).expect("fits");
        }

        let bit_count = writer.len_bits() as u32;
        Ok((writer.into_inner(), bit_count))
    }
}

/// One side of a delta comparison.
#[derive(Debug, Clone, Copy)]
pub struct SnapshotDeltaSide<'a> {
    pub snapshot_id: u64,
    /// Ascending, duplicate-free entity indices to compare, which is how a
    /// per-receiver visibility set narrows the snapshot. `None` compares every
    /// valid entity in the snapshot.
    pub transmit: Option<&'a [u32]>,
}

impl SnapshotDeltaSide<'_> {
    pub fn whole(snapshot_id: u64) -> Self {
        Self {
            snapshot_id,
            transmit: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    Missing(u64),
    TooManyActive,
    InvalidMaxEntities(u32),
    TooManyEntities(u64),
    InvalidEntityIndex(u32),
    InvalidSerialNumber(i32),
    InvalidClassId(u32),
    DuplicateEntity(u32),
    EntityOrder { previous: u32, current: u32 },
    InvalidDeleteSlot(u32),
    MissingEntity(u32),
    MissingDelete(u32),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing(id) => write!(formatter, "snapshot {id} is not registered"),
            Self::TooManyActive => write!(formatter, "snapshot registry is full"),
            Self::InvalidMaxEntities(count) => {
                write!(formatter, "snapshot entity capacity {count} is invalid")
            }
            Self::TooManyEntities(count) => {
                write!(formatter, "snapshot contains too many entities: {count}")
            }
            Self::InvalidEntityIndex(index) => {
                write!(formatter, "snapshot entity index {index} is invalid")
            }
            Self::InvalidSerialNumber(serial) => {
                write!(formatter, "snapshot entity serial {serial} is invalid")
            }
            Self::InvalidClassId(class_id) => {
                write!(formatter, "snapshot server class ID {class_id} is invalid")
            }
            Self::DuplicateEntity(index) => {
                write!(formatter, "snapshot entity {index} is duplicated")
            }
            Self::EntityOrder { previous, current } => write!(
                formatter,
                "snapshot entities are out of order: {current} follows {previous}"
            ),
            Self::InvalidDeleteSlot(slot) => {
                write!(formatter, "snapshot explicit-delete slot {slot} is invalid")
            }
            Self::MissingEntity(index) => {
                write!(formatter, "snapshot entity ordinal {index} is missing")
            }
            Self::MissingDelete(index) => {
                write!(
                    formatter,
                    "snapshot explicit-delete ordinal {index} is missing"
                )
            }
        }
    }
}

impl std::error::Error for SnapshotError {}

#[derive(Debug)]
struct Snapshot {
    tick: i32,
    max_entities: u32,
    entities: Vec<SnapshotEntity>,
    explicit_deletes: Vec<u32>,
}

impl Snapshot {
    fn summary(&self) -> SnapshotSummary {
        SnapshotSummary {
            tick: self.tick,
            max_entities: self.max_entities,
            valid_entity_count: self.entities.len() as u32,
            explicit_delete_count: self.explicit_deletes.len() as u32,
        }
    }
}

#[derive(Debug)]
pub struct SnapshotRegistry {
    next_id: u64,
    snapshots: HashMap<u64, Snapshot>,
    pending_deletes: Vec<u32>,
}

impl Default for SnapshotRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotRegistry {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            snapshots: HashMap::new(),
            pending_deletes: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        *self = Self::new();
    }

    pub fn queue_explicit_delete(&mut self, slot: u32) -> std::result::Result<bool, SnapshotError> {
        if slot as usize >= MAX_SNAPSHOT_ENTITIES {
            return Err(SnapshotError::InvalidDeleteSlot(slot));
        }
        if self.pending_deletes.contains(&slot) {
            return Ok(false);
        }
        self.pending_deletes.push(slot);
        Ok(true)
    }

    pub fn create(
        &mut self,
        tick: i32,
        max_entities: u32,
        entities: &[SnapshotEntity],
    ) -> std::result::Result<(u64, SnapshotSummary), SnapshotError> {
        if max_entities == 0 || max_entities as usize > MAX_SNAPSHOT_ENTITIES {
            return Err(SnapshotError::InvalidMaxEntities(max_entities));
        }
        if entities.len() > max_entities as usize {
            return Err(SnapshotError::TooManyEntities(entities.len() as u64));
        }
        if self.snapshots.len() >= MAX_ACTIVE_SNAPSHOTS {
            return Err(SnapshotError::TooManyActive);
        }
        let mut previous = None;
        for entity in entities {
            if entity.entity_index >= max_entities {
                return Err(SnapshotError::InvalidEntityIndex(entity.entity_index));
            }
            if entity.serial_number < 0 {
                return Err(SnapshotError::InvalidSerialNumber(entity.serial_number));
            }
            if entity.class_id as usize >= MAX_SERVER_CLASSES {
                return Err(SnapshotError::InvalidClassId(entity.class_id));
            }
            if let Some(previous) = previous {
                if entity.entity_index == previous {
                    return Err(SnapshotError::DuplicateEntity(entity.entity_index));
                }
                if entity.entity_index < previous {
                    return Err(SnapshotError::EntityOrder {
                        previous,
                        current: entity.entity_index,
                    });
                }
            }
            previous = Some(entity.entity_index);
        }

        let id = loop {
            let candidate = self.next_id;
            self.next_id = self.next_id.wrapping_add(1).max(1);
            if candidate != 0 && !self.snapshots.contains_key(&candidate) {
                break candidate;
            }
        };
        let snapshot = Snapshot {
            tick,
            max_entities,
            entities: entities.to_vec(),
            explicit_deletes: std::mem::take(&mut self.pending_deletes),
        };
        let summary = snapshot.summary();
        self.snapshots.insert(id, snapshot);
        Ok((id, summary))
    }

    pub fn remove(&mut self, id: u64) -> bool {
        self.snapshots.remove(&id).is_some()
    }

    pub fn summary(&self, id: u64) -> std::result::Result<SnapshotSummary, SnapshotError> {
        Ok(self
            .snapshots
            .get(&id)
            .ok_or(SnapshotError::Missing(id))?
            .summary())
    }

    pub fn entity(
        &self,
        id: u64,
        ordinal: u32,
    ) -> std::result::Result<SnapshotEntity, SnapshotError> {
        self.snapshots
            .get(&id)
            .ok_or(SnapshotError::Missing(id))?
            .entities
            .get(ordinal as usize)
            .copied()
            .ok_or(SnapshotError::MissingEntity(ordinal))
    }

    pub fn explicit_delete(
        &self,
        id: u64,
        ordinal: u32,
    ) -> std::result::Result<u32, SnapshotError> {
        self.snapshots
            .get(&id)
            .ok_or(SnapshotError::Missing(id))?
            .explicit_deletes
            .get(ordinal as usize)
            .copied()
            .ok_or(SnapshotError::MissingDelete(ordinal))
    }

    /// Resolves one comparison side into ascending `(index, identity)` pairs.
    ///
    /// An index that the snapshot has no entity for resolves to `None`, which
    /// mirrors the legacy classless snapshot entry. The caller decides whether
    /// that is usable, because only the newer side needs a class to write.
    fn delta_side(
        &self,
        side: SnapshotDeltaSide<'_>,
    ) -> std::result::Result<Vec<(u32, Option<SnapshotEntity>)>, SnapshotError> {
        let snapshot = self
            .snapshots
            .get(&side.snapshot_id)
            .ok_or(SnapshotError::Missing(side.snapshot_id))?;
        let Some(transmit) = side.transmit else {
            return Ok(snapshot
                .entities
                .iter()
                .map(|entity| (entity.entity_index, Some(*entity)))
                .collect());
        };

        if transmit.len() > MAX_SNAPSHOT_ENTITIES {
            return Err(SnapshotError::TooManyEntities(transmit.len() as u64));
        }
        let mut resolved = Vec::with_capacity(transmit.len());
        let mut previous: Option<u32> = None;
        for &index in transmit {
            if index as usize >= MAX_SNAPSHOT_ENTITIES {
                return Err(SnapshotError::InvalidEntityIndex(index));
            }
            if let Some(previous) = previous {
                if index == previous {
                    return Err(SnapshotError::DuplicateEntity(index));
                }
                if index < previous {
                    return Err(SnapshotError::EntityOrder {
                        previous,
                        current: index,
                    });
                }
            }
            previous = Some(index);
            let identity = snapshot
                .entities
                .binary_search_by_key(&index, |entity| entity.entity_index)
                .ok()
                .map(|ordinal| snapshot.entities[ordinal]);
            resolved.push((index, identity));
        }
        Ok(resolved)
    }

    /// Classifies every entity index that differs between two snapshots.
    ///
    /// `from` is `None` for a full update, where every entity in `to` has to be
    /// created. Entries come back in ascending entity-index order, which is the
    /// order the wire format requires, and there is at most one entry per index.
    pub fn delta(
        &self,
        from: Option<SnapshotDeltaSide<'_>>,
        to: SnapshotDeltaSide<'_>,
    ) -> std::result::Result<Vec<SnapshotDeltaEntry>, SnapshotError> {
        let new_side = self.delta_side(to)?;
        let old_side = match from {
            Some(from) => self.delta_side(from)?,
            None => Vec::new(),
        };

        let enter = |index: u32, identity: Option<SnapshotEntity>, recreated: bool| {
            // The newer side always has to name a class, because creating the
            // entity from its baseline requires one.
            identity
                .map(|entity| SnapshotDeltaEntry {
                    entity_index: index,
                    kind: SnapshotDeltaKind::EnterPvs,
                    class_id: entity.class_id,
                    serial_number: entity.serial_number,
                    recreated,
                })
                .ok_or(SnapshotError::MissingEntity(index))
        };
        let leave = |index: u32, identity: Option<SnapshotEntity>| SnapshotDeltaEntry {
            entity_index: index,
            kind: SnapshotDeltaKind::LeavePvs,
            // A classless older entry leaves the removal header to the caller's
            // own packed state, which is where the legacy writer reads it from.
            class_id: identity.map_or(0, |entity| entity.class_id),
            serial_number: identity.map_or(-1, |entity| entity.serial_number),
            recreated: false,
        };

        let mut entries = Vec::with_capacity(old_side.len().max(new_side.len()));
        let mut old = old_side.into_iter().peekable();
        let mut new = new_side.into_iter().peekable();

        // Both sides are ascending and duplicate-free, so one merge walk visits
        // each index exactly once and leaves the output in wire order.
        loop {
            let ordering = match (old.peek(), new.peek()) {
                (None, None) => break,
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some((old_index, _)), Some((new_index, _))) => old_index.cmp(new_index),
            };

            match ordering {
                std::cmp::Ordering::Less => {
                    let (index, identity) = old.next().expect("peeked");
                    entries.push(leave(index, identity));
                }
                std::cmp::Ordering::Greater => {
                    let (index, identity) = new.next().expect("peeked");
                    entries.push(enter(index, identity, false)?);
                }
                std::cmp::Ordering::Equal => {
                    let (_, old_identity) = old.next().expect("peeked");
                    let (index, new_identity) = new.next().expect("peeked");
                    // A reused index carrying a new serial is a different
                    // entity, so it cannot delta against the old payload. A
                    // changed class without a changed serial is a protocol
                    // inconsistency, and a recreate is the safe reading. An
                    // older side with no class has nothing to delta against.
                    let recreated = match (old_identity, new_identity) {
                        (Some(old_entity), Some(new_entity)) => {
                            old_entity.serial_number != new_entity.serial_number
                                || old_entity.class_id != new_entity.class_id
                        }
                        _ => true,
                    };
                    if recreated {
                        entries.push(enter(index, new_identity, true)?);
                    } else {
                        let entity = new_identity.ok_or(SnapshotError::MissingEntity(index))?;
                        entries.push(SnapshotDeltaEntry {
                            entity_index: index,
                            kind: SnapshotDeltaKind::DeltaCandidate,
                            class_id: entity.class_id,
                            serial_number: entity.serial_number,
                            recreated: false,
                        });
                    }
                }
            }
        }

        Ok(entries)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_messages: usize,
    pub max_string_bytes: usize,
    pub max_payload_bits: usize,
    pub max_classes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_messages: 1_000_000,
            max_string_bytes: 4096,
            max_payload_bits: 64 * 1024 * 1024 * 8,
            max_classes: 4096,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ParseOptions {
    pub network_protocol: i32,
    pub required_network_protocol: Option<i32>,
    /// Width of the leading message identifier. Current protocol 25 uses six;
    /// early protocol-7 demo streams use five.
    pub message_type_bits: u32,
    /// Decode fields compiled under `REPLAY_ENABLED` in the native engine.
    pub replay_enabled: bool,
    /// Decode fields present only in Xbox 360 network messages.
    pub xbox_360: bool,
    pub limits: Limits,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            network_protocol: NETWORK_PROTOCOL,
            required_network_protocol: Some(NETWORK_PROTOCOL),
            message_type_bits: MESSAGE_TYPE_BITS,
            replay_enabled: false,
            xbox_360: false,
            limits: Limits::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitSpan {
    pub start_bit: usize,
    pub bit_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MessageKind {
    Nop = 0,
    Disconnect = 1,
    File = 2,
    Tick = 3,
    StringCommand = 4,
    SetConVar = 5,
    SignonState = 6,
    Print = 7,
    ServerInfo = 8,
    SendTable = 9,
    ClassInfo = 10,
    SetPause = 11,
    CreateStringTable = 12,
    UpdateStringTable = 13,
    VoiceInit = 14,
    VoiceData = 15,
    Sounds = 17,
    SetView = 18,
    FixAngle = 19,
    CrosshairAngle = 20,
    BspDecal = 21,
    UserMessage = 23,
    EntityMessage = 24,
    GameEvent = 25,
    PacketEntities = 26,
    TempEntities = 27,
    Prefetch = 28,
    Menu = 29,
    GameEventList = 30,
    GetCvarValue = 31,
    CommandKeyValues = 32,
    SetPauseTimed = 33,
}

impl TryFrom<u8> for MessageKind {
    type Error = ();

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Nop,
            1 => Self::Disconnect,
            2 => Self::File,
            3 => Self::Tick,
            4 => Self::StringCommand,
            5 => Self::SetConVar,
            6 => Self::SignonState,
            7 => Self::Print,
            8 => Self::ServerInfo,
            9 => Self::SendTable,
            10 => Self::ClassInfo,
            11 => Self::SetPause,
            12 => Self::CreateStringTable,
            13 => Self::UpdateStringTable,
            14 => Self::VoiceInit,
            15 => Self::VoiceData,
            17 => Self::Sounds,
            18 => Self::SetView,
            19 => Self::FixAngle,
            20 => Self::CrosshairAngle,
            21 => Self::BspDecal,
            23 => Self::UserMessage,
            24 => Self::EntityMessage,
            25 => Self::GameEvent,
            26 => Self::PacketEntities,
            27 => Self::TempEntities,
            28 => Self::Prefetch,
            29 => Self::Menu,
            30 => Self::GameEventList,
            31 => Self::GetCvarValue,
            32 => Self::CommandKeyValues,
            33 => Self::SetPauseTimed,
            _ => return Err(()),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerInfo {
    pub protocol: i16,
    pub server_count: i32,
    pub is_hltv: bool,
    pub is_dedicated: bool,
    pub max_classes: u16,
    pub map_hash: Vec<u8>,
    pub player_slot: u8,
    pub max_clients: u8,
    pub tick_interval: f32,
    pub os: i8,
    pub game_directory: Vec<u8>,
    pub map_name: Vec<u8>,
    pub sky_name: Vec<u8>,
    pub host_name: Vec<u8>,
    pub is_replay: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerClass {
    pub id: u32,
    pub class_name: Vec<u8>,
    pub data_table_name: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageData {
    Nop,
    Disconnect(Vec<u8>),
    File {
        transfer_id: u32,
        name: Vec<u8>,
        requested: bool,
    },
    Tick {
        tick: i32,
        host_frame_time: Option<f32>,
        host_frame_time_std_deviation: Option<f32>,
    },
    StringCommand(Vec<u8>),
    SetConVar(Vec<(Vec<u8>, Vec<u8>)>),
    SignonState {
        state: u8,
        spawn_count: i32,
    },
    Print(Vec<u8>),
    ServerInfo(ServerInfo),
    SendTable {
        needs_decoder: bool,
        data: BitSpan,
    },
    ClassInfo {
        create_on_client: bool,
        classes: Vec<ServerClass>,
    },
    SetPause(bool),
    CreateStringTable {
        name: Vec<u8>,
        max_entries: u16,
        entry_count: u32,
        fixed_user_data: Option<(u32, u32)>,
        compressed: bool,
        data: BitSpan,
    },
    UpdateStringTable {
        table_id: u32,
        changed_entries: u16,
        data: BitSpan,
    },
    VoiceInit {
        codec: Vec<u8>,
        legacy_quality: u8,
        sample_rate: Option<i16>,
    },
    VoiceData {
        from_client: u8,
        proximity: bool,
        xuid: Option<u64>,
        data: BitSpan,
    },
    Sounds {
        reliable: bool,
        sound_count: u8,
        data: BitSpan,
    },
    SetView(u32),
    FixAngle {
        relative: bool,
        angles: [f32; 3],
    },
    CrosshairAngle([f32; 3]),
    BspDecal {
        position: [f32; 3],
        texture_index: u32,
        entity: Option<(u32, u32)>,
        low_priority: bool,
    },
    UserMessage {
        message_type: u8,
        data: BitSpan,
    },
    EntityMessage {
        entity_index: u32,
        class_id: u32,
        data: BitSpan,
    },
    GameEvent(BitSpan),
    PacketEntities {
        max_entries: u32,
        delta_from: Option<i32>,
        baseline: bool,
        updated_entries: u32,
        update_baseline: bool,
        data: BitSpan,
    },
    TempEntities {
        entry_count: u32,
        data: BitSpan,
    },
    Prefetch {
        sound_index: u32,
    },
    Menu {
        dialog_type: i16,
        data: BitSpan,
    },
    GameEventList {
        event_count: u32,
        data: BitSpan,
    },
    GetCvarValue {
        cookie: i32,
        name: Vec<u8>,
    },
    CommandKeyValues(BitSpan),
    SetPauseTimed {
        paused: bool,
        expires_at: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub kind: MessageKind,
    pub bit_range: Range<usize>,
    pub data: MessageData,
}

#[derive(Debug, Clone)]
pub struct MessageStream<'a> {
    bytes: &'a [u8],
    bit_len: usize,
    messages: Vec<Message>,
    trailing_bits: usize,
}

impl<'a> MessageStream<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_options(
            bytes,
            bytes.len().saturating_mul(8),
            ParseOptions::default(),
        )
    }

    pub fn parse_with_options(
        bytes: &'a [u8],
        bit_len: usize,
        options: ParseOptions,
    ) -> Result<Self> {
        validate_protocol(options)?;
        let mut reader = BitReader::with_bit_len(bytes, bit_len)?;
        let mut messages = Vec::new();
        while reader.remaining() >= options.message_type_bits as usize {
            if messages.len() >= options.limits.max_messages {
                return Err(Error::MessageLimitExceeded(options.limits.max_messages));
            }
            let start = reader.position();
            let raw_kind = read_u8(&mut reader, options.message_type_bits)?;
            let kind = MessageKind::try_from(raw_kind)
                .map_err(|()| Error::InvalidMessageType(raw_kind))?;
            let data = read_message(&mut reader, kind, options).map_err(|source| {
                Error::MessageDecode {
                    direction: "server",
                    kind: raw_kind,
                    index: messages.len(),
                    previous_kind: messages.last().map(|message: &Message| message.kind as u8),
                    previous_range: messages
                        .last()
                        .map(|message: &Message| message.bit_range.clone()),
                    recent: messages
                        .iter()
                        .rev()
                        .take(8)
                        .map(|message| (message.kind as u8, message.bit_range.clone()))
                        .collect(),
                    bit_offset: start,
                    source: Box::new(source),
                }
            })?;
            messages.push(Message {
                kind,
                bit_range: start..reader.position(),
                data,
            });
        }
        let trailing_bits = reader.remaining();
        Ok(Self {
            bytes,
            bit_len,
            messages,
            trailing_bits,
        })
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn trailing_bits(&self) -> usize {
        self.trailing_bits
    }

    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    pub fn original_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

/// Client-to-server message identifiers from `common/protocol.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ClientMessageKind {
    Nop = 0,
    Disconnect = 1,
    File = 2,
    Tick = 3,
    StringCommand = 4,
    SetConVar = 5,
    SignonState = 6,
    ClientInfo = 8,
    Move = 9,
    VoiceData = 10,
    BaselineAck = 11,
    ListenEvents = 12,
    RespondCvarValue = 13,
    FileCrcCheck = 14,
    SaveReplay = 15,
    CommandKeyValues = 16,
    FileMd5Check = 17,
}

impl TryFrom<u8> for ClientMessageKind {
    type Error = ();

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            0 => Self::Nop,
            1 => Self::Disconnect,
            2 => Self::File,
            3 => Self::Tick,
            4 => Self::StringCommand,
            5 => Self::SetConVar,
            6 => Self::SignonState,
            8 => Self::ClientInfo,
            9 => Self::Move,
            10 => Self::VoiceData,
            11 => Self::BaselineAck,
            12 => Self::ListenEvents,
            13 => Self::RespondCvarValue,
            14 => Self::FileCrcCheck,
            15 => Self::SaveReplay,
            16 => Self::CommandKeyValues,
            17 => Self::FileMd5Check,
            _ => return Err(()),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCrcCheck {
    pub reserved: bool,
    pub path_id: Vec<u8>,
    pub filename: Vec<u8>,
    pub is_new_format: bool,
    pub md5: Option<[u8; 16]>,
    pub legacy_crc: Option<u32>,
    pub crc_ios: u32,
    pub file_hash_type: u32,
    pub file_len: Option<u32>,
    pub pack_file_number: Option<u32>,
    pub pack_file_id: Option<u32>,
    pub file_fraction: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileMd5Check {
    pub reserved: bool,
    pub path_id: Vec<u8>,
    pub filename: Vec<u8>,
    pub md5: [u8; 16],
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClientMessageData {
    /// One of the direction-independent `net_*` message layouts (IDs 0-6).
    Common(MessageData),
    ClientInfo {
        server_count: i32,
        send_table_crc: u32,
        is_hltv: bool,
        friends_id: u32,
        friends_name: Vec<u8>,
        custom_files: [Option<u32>; 4],
        is_replay: Option<bool>,
    },
    Move {
        new_commands: u8,
        backup_commands: u8,
        data: BitSpan,
    },
    VoiceData {
        xuid: Option<u64>,
        data: BitSpan,
    },
    BaselineAck {
        tick: i32,
        baseline: bool,
    },
    ListenEvents([u32; 16]),
    RespondCvarValue {
        cookie: i32,
        status: i8,
        name: Vec<u8>,
        value: Vec<u8>,
    },
    FileCrcCheck(FileCrcCheck),
    SaveReplay {
        filename: Vec<u8>,
        start_send_byte: u8,
        post_death_record_time: f32,
    },
    CommandKeyValues(BitSpan),
    FileMd5Check(FileMd5Check),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientMessage {
    pub kind: ClientMessageKind,
    pub bit_range: Range<usize>,
    pub data: ClientMessageData,
}

#[derive(Debug, Clone)]
pub struct ClientMessageStream<'a> {
    bytes: &'a [u8],
    bit_len: usize,
    messages: Vec<ClientMessage>,
    trailing_bits: usize,
}

impl<'a> ClientMessageStream<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_options(
            bytes,
            bytes.len().saturating_mul(8),
            ParseOptions::default(),
        )
    }

    pub fn parse_with_options(
        bytes: &'a [u8],
        bit_len: usize,
        options: ParseOptions,
    ) -> Result<Self> {
        validate_protocol(options)?;
        let mut reader = BitReader::with_bit_len(bytes, bit_len)?;
        let mut messages = Vec::new();
        while reader.remaining() >= options.message_type_bits as usize {
            if messages.len() >= options.limits.max_messages {
                return Err(Error::MessageLimitExceeded(options.limits.max_messages));
            }
            let start = reader.position();
            let raw_kind = read_u8(&mut reader, options.message_type_bits)?;
            let kind = ClientMessageKind::try_from(raw_kind)
                .map_err(|()| Error::InvalidClientMessageType(raw_kind))?;
            if kind == ClientMessageKind::SaveReplay && !options.replay_enabled {
                return Err(Error::InvalidClientMessageType(raw_kind));
            }
            let data = read_client_message(&mut reader, kind, options).map_err(|source| {
                Error::MessageDecode {
                    direction: "client",
                    kind: raw_kind,
                    index: messages.len(),
                    previous_kind: messages
                        .last()
                        .map(|message: &ClientMessage| message.kind as u8),
                    previous_range: messages
                        .last()
                        .map(|message: &ClientMessage| message.bit_range.clone()),
                    recent: messages
                        .iter()
                        .rev()
                        .take(8)
                        .map(|message| (message.kind as u8, message.bit_range.clone()))
                        .collect(),
                    bit_offset: start,
                    source: Box::new(source),
                }
            })?;
            messages.push(ClientMessage {
                kind,
                bit_range: start..reader.position(),
                data,
            });
        }
        let trailing_bits = reader.remaining();
        Ok(Self {
            bytes,
            bit_len,
            messages,
            trailing_bits,
        })
    }

    pub fn messages(&self) -> &[ClientMessage] {
        &self.messages
    }

    pub fn trailing_bits(&self) -> usize {
        self.trailing_bits
    }

    pub fn bit_len(&self) -> usize {
        self.bit_len
    }

    pub fn original_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    UnsupportedNetworkProtocol(i32),
    InvalidMessageType(u8),
    InvalidClientMessageType(u8),
    InvalidMessageTypeBits(u32),
    MessageLimitExceeded(usize),
    UnterminatedString {
        offset: usize,
        limit: usize,
    },
    InvalidLength {
        what: &'static str,
        value: i64,
    },
    PayloadLimitExceeded {
        bits: usize,
        limit: usize,
    },
    ClassLimitExceeded {
        count: usize,
        limit: usize,
    },
    InvalidVarint,
    InvalidPathIdCode(u8),
    InvalidFilenamePrefixCode(u8),
    MessageDecode {
        direction: &'static str,
        kind: u8,
        index: usize,
        previous_kind: Option<u8>,
        previous_range: Option<Range<usize>>,
        recent: Vec<(u8, Range<usize>)>,
        bit_offset: usize,
        source: Box<Error>,
    },
    ServerProtocolMismatch {
        stream: i32,
        message: i16,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::UnsupportedNetworkProtocol(value) => {
                write!(f, "unsupported network protocol {value}")
            }
            Self::InvalidMessageType(value) => write!(f, "invalid server message type {value}"),
            Self::InvalidClientMessageType(value) => {
                write!(f, "invalid client message type {value}")
            }
            Self::InvalidMessageTypeBits(value) => {
                write!(f, "invalid network message type width {value}")
            }
            Self::MessageLimitExceeded(limit) => {
                write!(f, "network message count exceeds {limit}")
            }
            Self::UnterminatedString { offset, limit } => {
                write!(
                    f,
                    "unterminated network string at bit {offset} (limit {limit})"
                )
            }
            Self::InvalidLength { what, value } => {
                write!(f, "invalid network {what} {value}")
            }
            Self::PayloadLimitExceeded { bits, limit } => {
                write!(
                    f,
                    "network payload has {bits} bits, exceeding limit {limit}"
                )
            }
            Self::ClassLimitExceeded { count, limit } => {
                write!(f, "network class count {count} exceeds limit {limit}")
            }
            Self::InvalidVarint => write!(f, "invalid network varint32"),
            Self::InvalidPathIdCode(value) => write!(f, "invalid file path-ID code {value}"),
            Self::InvalidFilenamePrefixCode(value) => {
                write!(f, "invalid file-name prefix code {value}")
            }
            Self::MessageDecode {
                direction,
                kind,
                index,
                previous_kind,
                previous_range,
                recent,
                bit_offset,
                source,
            } => write!(
                f,
                "failed decoding {direction} message #{index} type {kind} at bit {bit_offset} (previous type {previous_kind:?} at {previous_range:?}; recent reversed {recent:?}): {source}"
            ),
            Self::ServerProtocolMismatch { stream, message } => write!(
                f,
                "server-info protocol {message} does not match stream protocol {stream}"
            ),
        }
    }
}

impl std::error::Error for Error {}

impl From<source_binary::Error> for Error {
    fn from(value: source_binary::Error) -> Self {
        Self::Binary(value)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn validate_protocol(options: ParseOptions) -> Result<()> {
    if !(1..=8).contains(&options.message_type_bits) {
        return Err(Error::InvalidMessageTypeBits(options.message_type_bits));
    }
    if options
        .required_network_protocol
        .is_some_and(|required| options.network_protocol != required)
    {
        return Err(Error::UnsupportedNetworkProtocol(options.network_protocol));
    }
    Ok(())
}

fn read_client_message(
    reader: &mut BitReader<'_>,
    kind: ClientMessageKind,
    options: ParseOptions,
) -> Result<ClientMessageData> {
    let limits = options.limits;
    if (kind as u8) <= ClientMessageKind::SignonState as u8 {
        let common_kind = MessageKind::try_from(kind as u8)
            .map_err(|()| Error::InvalidClientMessageType(kind as u8))?;
        return read_message(reader, common_kind, options).map(ClientMessageData::Common);
    }

    Ok(match kind {
        ClientMessageKind::Nop
        | ClientMessageKind::Disconnect
        | ClientMessageKind::File
        | ClientMessageKind::Tick
        | ClientMessageKind::StringCommand
        | ClientMessageKind::SetConVar
        | ClientMessageKind::SignonState => unreachable!("common messages returned above"),
        ClientMessageKind::ClientInfo => {
            let server_count = read_i32(reader, 32)?;
            let send_table_crc = read_u32(reader, 32)?;
            let is_hltv = reader.read_bool()?;
            let friends_id = read_u32(reader, 32)?;
            let friends_name = read_string(reader, limits)?;
            let mut custom_files = [None; 4];
            for file in &mut custom_files {
                if reader.read_bool()? {
                    *file = Some(read_u32(reader, 32)?);
                }
            }
            let is_replay = options
                .replay_enabled
                .then(|| reader.read_bool())
                .transpose()?;
            ClientMessageData::ClientInfo {
                server_count,
                send_table_crc,
                is_hltv,
                friends_id,
                friends_name,
                custom_files,
                is_replay,
            }
        }
        ClientMessageKind::Move => {
            let new_commands = read_u8(reader, 4)?;
            let backup_commands = read_u8(reader, 3)?;
            let length = usize::from(read_u16(reader, 16)?);
            ClientMessageData::Move {
                new_commands,
                backup_commands,
                data: take_payload(reader, length, limits)?,
            }
        }
        ClientMessageKind::VoiceData => {
            let length = usize::from(read_u16(reader, 16)?);
            let xuid = options.xbox_360.then(|| read_u64(reader, 64)).transpose()?;
            ClientMessageData::VoiceData {
                xuid,
                data: take_payload(reader, length, limits)?,
            }
        }
        ClientMessageKind::BaselineAck => ClientMessageData::BaselineAck {
            tick: read_i32(reader, 32)?,
            baseline: reader.read_bool()?,
        },
        ClientMessageKind::ListenEvents => {
            let mut events = [0; 16];
            for event_word in &mut events {
                *event_word = read_u32(reader, 32)?;
            }
            ClientMessageData::ListenEvents(events)
        }
        ClientMessageKind::RespondCvarValue => ClientMessageData::RespondCvarValue {
            cookie: read_i32(reader, 32)?,
            status: read_i32(reader, 4)? as i8,
            name: read_string(reader, limits)?,
            value: read_string(reader, limits)?,
        },
        ClientMessageKind::FileCrcCheck => {
            ClientMessageData::FileCrcCheck(read_file_crc_check(reader, limits)?)
        }
        ClientMessageKind::SaveReplay => ClientMessageData::SaveReplay {
            filename: read_string(reader, limits)?,
            // The native writer intentionally passes sizeof(int), i.e. four bits.
            start_send_byte: read_u8(reader, 4)?,
            post_death_record_time: read_f32(reader)?,
        },
        ClientMessageKind::CommandKeyValues => {
            ClientMessageData::CommandKeyValues(read_command_keyvalues(reader, limits)?)
        }
        ClientMessageKind::FileMd5Check => {
            ClientMessageData::FileMd5Check(read_file_md5_check(reader, limits)?)
        }
    })
}

fn read_file_crc_check(reader: &mut BitReader<'_>, limits: Limits) -> Result<FileCrcCheck> {
    let reserved = reader.read_bool()?;
    let path_id = read_path_id(reader, limits)?;
    let prefix_code = read_u8(reader, 3)?;
    let first = read_u8(reader, 8)?;
    let is_new_format = first == 1;
    let filename_tail = read_string(reader, limits)?;
    let filename = if is_new_format {
        apply_filename_prefix(prefix_code, filename_tail)?
    } else {
        let mut filename = Vec::with_capacity(filename_tail.len().saturating_add(1));
        if first != 0 {
            filename.push(first);
            filename.extend_from_slice(&filename_tail);
        }
        if filename.len() > limits.max_string_bytes {
            return Err(Error::UnterminatedString {
                offset: reader.position(),
                limit: limits.max_string_bytes,
            });
        }
        apply_filename_prefix(prefix_code, filename)?
    };

    if is_new_format {
        Ok(FileCrcCheck {
            reserved,
            path_id,
            filename,
            is_new_format,
            md5: Some(read_array_16(reader)?),
            legacy_crc: None,
            crc_ios: read_u32(reader, 32)?,
            file_hash_type: read_u32(reader, 32)?,
            file_len: Some(read_u32(reader, 32)?),
            pack_file_number: Some(read_u32(reader, 32)?),
            pack_file_id: Some(read_u32(reader, 32)?),
            file_fraction: Some(read_u32(reader, 32)?),
        })
    } else {
        Ok(FileCrcCheck {
            reserved,
            path_id,
            filename,
            is_new_format,
            md5: None,
            legacy_crc: Some(read_u32(reader, 32)?),
            crc_ios: read_u32(reader, 32)?,
            file_hash_type: read_u32(reader, 32)?,
            file_len: None,
            pack_file_number: None,
            pack_file_id: None,
            file_fraction: None,
        })
    }
}

fn read_file_md5_check(reader: &mut BitReader<'_>, limits: Limits) -> Result<FileMd5Check> {
    let reserved = reader.read_bool()?;
    let path_id = read_path_id(reader, limits)?;
    let prefix_code = read_u8(reader, 3)?;
    let filename = apply_filename_prefix(prefix_code, read_string(reader, limits)?)?;
    Ok(FileMd5Check {
        reserved,
        path_id,
        filename,
        md5: read_array_16(reader)?,
    })
}

fn read_path_id(reader: &mut BitReader<'_>, limits: Limits) -> Result<Vec<u8>> {
    match read_u8(reader, 2)? {
        0 => read_string(reader, limits),
        1 => Ok(b"GAME".to_vec()),
        2 => Ok(b"MOD".to_vec()),
        code => Err(Error::InvalidPathIdCode(code)),
    }
}

fn apply_filename_prefix(code: u8, tail: Vec<u8>) -> Result<Vec<u8>> {
    let prefix: &[u8] = match code {
        0 => return Ok(tail),
        1 => b"materials",
        2 => b"models",
        3 => b"sounds",
        4 => b"scripts",
        _ => return Err(Error::InvalidFilenamePrefixCode(code)),
    };
    let mut filename = Vec::with_capacity(prefix.len() + 1 + tail.len());
    filename.extend_from_slice(prefix);
    filename.push(b'/');
    filename.extend_from_slice(&tail);
    Ok(filename)
}

fn read_array_16(reader: &mut BitReader<'_>) -> Result<[u8; 16]> {
    let mut bytes = [0; 16];
    for byte in &mut bytes {
        *byte = read_u8(reader, 8)?;
    }
    Ok(bytes)
}

fn read_command_keyvalues(reader: &mut BitReader<'_>, limits: Limits) -> Result<BitSpan> {
    let byte_length = read_i32(reader, 32)?;
    let byte_length = valid_length("KeyValues byte length", i64::from(byte_length))?;
    if byte_length == 0 {
        return Err(Error::InvalidLength {
            what: "KeyValues byte length",
            value: 0,
        });
    }
    take_payload(reader, byte_length.saturating_mul(8), limits)
}

fn read_message(
    reader: &mut BitReader<'_>,
    kind: MessageKind,
    options: ParseOptions,
) -> Result<MessageData> {
    let limits = options.limits;
    Ok(match kind {
        MessageKind::Nop => MessageData::Nop,
        MessageKind::Disconnect => MessageData::Disconnect(read_string(reader, limits)?),
        MessageKind::File => MessageData::File {
            transfer_id: read_u32(reader, 32)?,
            name: read_string(reader, limits)?,
            requested: reader.read_bool()?,
        },
        MessageKind::Tick => {
            let tick = read_i32(reader, 32)?;
            let timing = (options.network_protocol > 10).then(|| {
                Ok::<_, Error>((
                    read_u16(reader, 16)? as f32 / 100_000.0,
                    read_u16(reader, 16)? as f32 / 100_000.0,
                ))
            });
            let timing = timing.transpose()?;
            MessageData::Tick {
                tick,
                host_frame_time: timing.map(|value| value.0),
                host_frame_time_std_deviation: timing.map(|value| value.1),
            }
        }
        MessageKind::StringCommand => MessageData::StringCommand(read_string(reader, limits)?),
        MessageKind::SetConVar => {
            let count = usize::from(read_u8(reader, 8)?);
            let mut values = Vec::with_capacity(count);
            for _ in 0..count {
                values.push((read_string(reader, limits)?, read_string(reader, limits)?));
            }
            MessageData::SetConVar(values)
        }
        MessageKind::SignonState => MessageData::SignonState {
            state: read_u8(reader, 8)?,
            spawn_count: read_i32(reader, 32)?,
        },
        MessageKind::Print => MessageData::Print(read_string(reader, limits)?),
        MessageKind::ServerInfo => MessageData::ServerInfo(read_server_info(reader, options)?),
        MessageKind::SendTable => {
            let needs_decoder = reader.read_bool()?;
            let length = read_i16(reader)?;
            let length = valid_length("send-table length", i64::from(length))?;
            MessageData::SendTable {
                needs_decoder,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::ClassInfo => read_class_info(reader, limits)?,
        MessageKind::SetPause => MessageData::SetPause(reader.read_bool()?),
        MessageKind::CreateStringTable => read_create_string_table(reader, options)?,
        MessageKind::UpdateStringTable => {
            let table_id = read_u32(reader, if options.network_protocol <= 7 { 4 } else { 5 })?;
            let changed_entries = if reader.read_bool()? {
                read_u16(reader, 16)?
            } else {
                1
            };
            let length = read_u32(
                reader,
                if options.network_protocol <= 7 {
                    16
                } else {
                    20
                },
            )? as usize;
            MessageData::UpdateStringTable {
                table_id,
                changed_entries,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::VoiceInit => {
            let codec = read_string(reader, limits)?;
            let legacy_quality = read_u8(reader, 8)?;
            let sample_rate = if legacy_quality == 255 {
                Some(read_i16(reader)?)
            } else {
                None
            };
            MessageData::VoiceInit {
                codec,
                legacy_quality,
                sample_rate,
            }
        }
        MessageKind::VoiceData => {
            let from_client = read_u8(reader, 8)?;
            let proximity = read_u8(reader, 8)? != 0;
            let length = usize::from(read_u16(reader, 16)?);
            let xuid = options.xbox_360.then(|| read_u64(reader, 64)).transpose()?;
            MessageData::VoiceData {
                from_client,
                proximity,
                xuid,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::Sounds => {
            let reliable = reader.read_bool()?;
            let (sound_count, length) = if reliable {
                (1, usize::from(read_u8(reader, 8)?))
            } else {
                (read_u8(reader, 8)?, usize::from(read_u16(reader, 16)?))
            };
            MessageData::Sounds {
                reliable,
                sound_count,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::SetView => MessageData::SetView(read_u32(reader, 11)?),
        MessageKind::FixAngle => MessageData::FixAngle {
            relative: reader.read_bool()?,
            angles: read_angles(reader)?,
        },
        MessageKind::CrosshairAngle => MessageData::CrosshairAngle(read_angles(reader)?),
        MessageKind::BspDecal => {
            let position = read_coord_vector(reader)?;
            let texture_index = read_u32(reader, 9)?;
            let entity = if reader.read_bool()? {
                Some((
                    read_u32(reader, 11)?,
                    read_u32(
                        reader,
                        if options.network_protocol <= 7 {
                            11
                        } else {
                            13
                        },
                    )?,
                ))
            } else {
                None
            };
            MessageData::BspDecal {
                position,
                texture_index,
                entity,
                low_priority: reader.read_bool()?,
            }
        }
        MessageKind::UserMessage => {
            let message_type = read_u8(reader, 8)?;
            let length = read_u32(reader, 11)? as usize;
            MessageData::UserMessage {
                message_type,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::EntityMessage => {
            let entity_index = read_u32(reader, 11)?;
            let class_id = read_u32(reader, 9)?;
            let length = read_u32(reader, 11)? as usize;
            MessageData::EntityMessage {
                entity_index,
                class_id,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::GameEvent => {
            let length = read_u32(reader, 11)? as usize;
            MessageData::GameEvent(take_payload(reader, length, limits)?)
        }
        MessageKind::PacketEntities => {
            let max_entries = read_u32(reader, 11)?;
            let delta_from = if reader.read_bool()? {
                Some(read_i32(reader, 32)?)
            } else {
                None
            };
            let baseline = reader.read_bool()?;
            let updated_entries = read_u32(reader, 11)?;
            let length = read_u32(reader, 20)? as usize;
            let update_baseline = reader.read_bool()?;
            MessageData::PacketEntities {
                max_entries,
                delta_from,
                baseline,
                updated_entries,
                update_baseline,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::TempEntities => {
            let entry_count = read_u32(reader, 8)?;
            let length = if options.network_protocol > 23 {
                read_varint32(reader)? as usize
            } else {
                read_u32(reader, 17)? as usize
            };
            MessageData::TempEntities {
                entry_count,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::Prefetch => MessageData::Prefetch {
            sound_index: read_u32(
                reader,
                if options.network_protocol > 22 {
                    14
                } else {
                    13
                },
            )?,
        },
        MessageKind::Menu => {
            let dialog_type = read_i16(reader)?;
            let byte_length = usize::from(read_u16(reader, 16)?);
            MessageData::Menu {
                dialog_type,
                data: take_payload(reader, byte_length.saturating_mul(8), limits)?,
            }
        }
        MessageKind::GameEventList => {
            let event_count = read_u32(reader, 9)?;
            let length = read_u32(reader, 20)? as usize;
            MessageData::GameEventList {
                event_count,
                data: take_payload(reader, length, limits)?,
            }
        }
        MessageKind::GetCvarValue => MessageData::GetCvarValue {
            cookie: read_i32(reader, 32)?,
            name: read_string(reader, limits)?,
        },
        MessageKind::CommandKeyValues => {
            MessageData::CommandKeyValues(read_command_keyvalues(reader, limits)?)
        }
        MessageKind::SetPauseTimed => MessageData::SetPauseTimed {
            paused: reader.read_bool()?,
            expires_at: read_f32(reader)?,
        },
    })
}

fn read_server_info(reader: &mut BitReader<'_>, options: ParseOptions) -> Result<ServerInfo> {
    let protocol = read_i16(reader)?;
    if i32::from(protocol) != options.network_protocol {
        return Err(Error::ServerProtocolMismatch {
            stream: options.network_protocol,
            message: protocol,
        });
    }
    let server_count = read_i32(reader, 32)?;
    let is_hltv = reader.read_bool()?;
    let is_dedicated = reader.read_bool()?;
    let _legacy_client_crc = read_u32(reader, 32)?;
    let max_classes = read_u16(reader, 16)?;
    let hash_len = if options.network_protocol > 17 { 16 } else { 4 };
    let mut map_hash = Vec::with_capacity(hash_len);
    for _ in 0..hash_len {
        map_hash.push(read_u8(reader, 8)?);
    }
    let player_slot = read_u8(reader, 8)?;
    let max_clients = read_u8(reader, 8)?;
    let tick_interval = read_f32(reader)?;
    let os = read_u8(reader, 8)? as i8;
    let game_directory = read_string(reader, options.limits)?;
    let map_name = read_string(reader, options.limits)?;
    let sky_name = read_string(reader, options.limits)?;
    let host_name = read_string(reader, options.limits)?;
    let is_replay = (options.replay_enabled && options.network_protocol >= 16)
        .then(|| reader.read_bool())
        .transpose()?;
    Ok(ServerInfo {
        protocol,
        server_count,
        is_hltv,
        is_dedicated,
        max_classes,
        map_hash,
        player_slot,
        max_clients,
        tick_interval,
        os,
        game_directory,
        map_name,
        sky_name,
        host_name,
        is_replay,
    })
}

fn read_class_info(reader: &mut BitReader<'_>, limits: Limits) -> Result<MessageData> {
    let raw_count = read_i16(reader)?;
    let count = valid_length("server class count", i64::from(raw_count))?;
    if count > limits.max_classes {
        return Err(Error::ClassLimitExceeded {
            count,
            limit: limits.max_classes,
        });
    }
    let class_bits = floor_log2(count).saturating_add(1);
    let create_on_client = reader.read_bool()?;
    let mut classes = Vec::new();
    if !create_on_client {
        classes.reserve(count);
        for _ in 0..count {
            classes.push(ServerClass {
                id: read_u32(reader, class_bits)?,
                class_name: read_string(reader, limits)?,
                data_table_name: read_string(reader, limits)?,
            });
        }
    }
    Ok(MessageData::ClassInfo {
        create_on_client,
        classes,
    })
}

fn read_create_string_table(
    reader: &mut BitReader<'_>,
    options: ParseOptions,
) -> Result<MessageData> {
    if reader.peek_bits(8)? == u64::from(b':') {
        read_u8(reader, 8)?;
    }
    let name = read_string(reader, options.limits)?;
    let max_entries = read_u16(reader, 16)?;
    let entry_bits = floor_log2(usize::from(max_entries)).saturating_add(1);
    let entry_count = read_u32(reader, entry_bits)?;
    let length = if options.network_protocol > 23 {
        read_varint32(reader)? as usize
    } else {
        read_u32(reader, 20)? as usize
    };
    let fixed_user_data = if reader.read_bool()? {
        Some((read_u32(reader, 12)?, read_u32(reader, 4)?))
    } else {
        None
    };
    let compressed = if options.network_protocol > 14 {
        reader.read_bool()?
    } else {
        false
    };
    Ok(MessageData::CreateStringTable {
        name,
        max_entries,
        entry_count,
        fixed_user_data,
        compressed,
        data: take_payload(reader, length, options.limits)?,
    })
}

fn read_angles(reader: &mut BitReader<'_>) -> Result<[f32; 3]> {
    let mut values = [0.0; 3];
    for value in &mut values {
        *value = read_u16(reader, 16)? as f32 * (360.0 / 65_536.0);
    }
    Ok(values)
}

fn read_coord_vector(reader: &mut BitReader<'_>) -> Result<[f32; 3]> {
    let present = [
        reader.read_bool()?,
        reader.read_bool()?,
        reader.read_bool()?,
    ];
    let mut values = [0.0; 3];
    for (index, is_present) in present.into_iter().enumerate() {
        if is_present {
            values[index] = read_coord(reader)?;
        }
    }
    Ok(values)
}

fn read_coord(reader: &mut BitReader<'_>) -> Result<f32> {
    let has_integer = reader.read_bool()?;
    let has_fraction = reader.read_bool()?;
    if !has_integer && !has_fraction {
        return Ok(0.0);
    }
    let negative = reader.read_bool()?;
    let integer = if has_integer {
        read_u32(reader, 14)? + 1
    } else {
        0
    };
    let fraction = if has_fraction {
        read_u32(reader, 5)?
    } else {
        0
    };
    let value = integer as f32 + fraction as f32 * (1.0 / 32.0);
    Ok(if negative { -value } else { value })
}

fn read_string(reader: &mut BitReader<'_>, limits: Limits) -> Result<Vec<u8>> {
    let start = reader.position();
    let mut value = Vec::new();
    loop {
        let byte = read_u8(reader, 8)?;
        if byte == 0 {
            return Ok(value);
        }
        if value.len() == limits.max_string_bytes {
            return Err(Error::UnterminatedString {
                offset: start,
                limit: limits.max_string_bytes,
            });
        }
        value.push(byte);
    }
}

fn take_payload(reader: &mut BitReader<'_>, bits: usize, limits: Limits) -> Result<BitSpan> {
    if bits > limits.max_payload_bits {
        return Err(Error::PayloadLimitExceeded {
            bits,
            limit: limits.max_payload_bits,
        });
    }
    let start_bit = reader.position();
    reader.skip_bits(bits)?;
    Ok(BitSpan {
        start_bit,
        bit_len: bits,
    })
}

fn read_varint32(reader: &mut BitReader<'_>) -> Result<u32> {
    let mut value = 0u32;
    for index in 0..5 {
        let byte = read_u8(reader, 8)?;
        if index == 4 && byte > 0x0f {
            return Err(Error::InvalidVarint);
        }
        value |= u32::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(Error::InvalidVarint)
}

fn read_u8(reader: &mut BitReader<'_>, bits: u32) -> Result<u8> {
    Ok(reader.read_bits(bits)? as u8)
}

fn read_u16(reader: &mut BitReader<'_>, bits: u32) -> Result<u16> {
    Ok(reader.read_bits(bits)? as u16)
}

fn read_u32(reader: &mut BitReader<'_>, bits: u32) -> Result<u32> {
    Ok(reader.read_bits(bits)? as u32)
}

fn read_u64(reader: &mut BitReader<'_>, bits: u32) -> Result<u64> {
    reader.read_bits(bits).map_err(Into::into)
}

fn read_i16(reader: &mut BitReader<'_>) -> Result<i16> {
    Ok(read_u16(reader, 16)? as i16)
}

fn read_i32(reader: &mut BitReader<'_>, bits: u32) -> Result<i32> {
    let raw = read_u32(reader, bits)?;
    if bits == 32 {
        return Ok(raw as i32);
    }
    let sign = 1u32 << (bits - 1);
    Ok(if raw & sign == 0 {
        raw as i32
    } else {
        (raw | (!0u32 << bits)) as i32
    })
}

fn read_f32(reader: &mut BitReader<'_>) -> Result<f32> {
    Ok(f32::from_bits(read_u32(reader, 32)?))
}

fn valid_length(what: &'static str, value: i64) -> Result<usize> {
    usize::try_from(value).map_err(|_| Error::InvalidLength { what, value })
}

fn floor_log2(mut value: usize) -> u32 {
    let mut answer = 0;
    while value >> 1 != 0 {
        value >>= 1;
        answer += 1;
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_binary::BitWriter;

    fn bits(writer: &mut BitWriter, value: u64, count: u32) {
        writer.write_bits(value, count).unwrap();
    }

    fn string(writer: &mut BitWriter, value: &[u8]) {
        for byte in value.iter().copied().chain([0]) {
            bits(writer, u64::from(byte), 8);
        }
    }

    #[test]
    fn parses_common_protocol_25_messages() {
        let mut writer = BitWriter::new();
        bits(&mut writer, MessageKind::Nop as u64, 6);
        bits(&mut writer, MessageKind::Tick as u64, 6);
        bits(&mut writer, 42, 32);
        bits(&mut writer, 1500, 16);
        bits(&mut writer, 250, 16);
        bits(&mut writer, MessageKind::StringCommand as u64, 6);
        string(&mut writer, b"status");
        bits(&mut writer, MessageKind::SetConVar as u64, 6);
        bits(&mut writer, 1, 8);
        string(&mut writer, b"sv_cheats");
        string(&mut writer, b"0");
        bits(&mut writer, MessageKind::SignonState as u64, 6);
        bits(&mut writer, 6, 8);
        bits(&mut writer, 99, 32);

        let stream = MessageStream::parse_with_options(
            writer.as_slice(),
            writer.len_bits(),
            ParseOptions::default(),
        )
        .unwrap();
        assert_eq!(stream.messages().len(), 5);
        assert!(matches!(
            &stream.messages()[1].data,
            MessageData::Tick {
                tick: 42,
                host_frame_time: Some(value),
                ..
            } if (*value - 0.015).abs() < f32::EPSILON
        ));
        assert!(matches!(
            &stream.messages()[3].data,
            MessageData::SetConVar(values) if values[0].0 == b"sv_cheats" && values[0].1 == b"0"
        ));
        assert_eq!(stream.trailing_bits(), 0);
    }

    #[test]
    fn skips_bounded_length_delimited_payloads() {
        let mut writer = BitWriter::new();
        bits(&mut writer, MessageKind::UserMessage as u64, 6);
        bits(&mut writer, 7, 8);
        bits(&mut writer, 5, 11);
        bits(&mut writer, 0b1_1010, 5);
        bits(&mut writer, MessageKind::PacketEntities as u64, 6);
        bits(&mut writer, 12, 11);
        bits(&mut writer, 1, 1);
        bits(&mut writer, u32::MAX as u64, 32);
        bits(&mut writer, 1, 1);
        bits(&mut writer, 3, 11);
        bits(&mut writer, 3, 20);
        bits(&mut writer, 0, 1);
        bits(&mut writer, 0b101, 3);

        let stream = MessageStream::parse_with_options(
            writer.as_slice(),
            writer.len_bits(),
            ParseOptions::default(),
        )
        .unwrap();
        assert_eq!(stream.messages().len(), 2);
        assert!(matches!(
            stream.messages()[0].data,
            MessageData::UserMessage {
                message_type: 7,
                data: BitSpan { bit_len: 5, .. }
            }
        ));
        assert!(matches!(
            stream.messages()[1].data,
            MessageData::PacketEntities {
                delta_from: Some(-1),
                data: BitSpan { bit_len: 3, .. },
                ..
            }
        ));
    }

    #[test]
    fn parses_client_messages_and_platform_fields() {
        let mut writer = BitWriter::new();
        bits(&mut writer, ClientMessageKind::ClientInfo as u64, 6);
        bits(&mut writer, 7, 32);
        bits(&mut writer, 0x1234_5678, 32);
        bits(&mut writer, 1, 1);
        bits(&mut writer, 765, 32);
        string(&mut writer, b"Friend");
        bits(&mut writer, 1, 1);
        bits(&mut writer, 0xaabb_ccdd, 32);
        bits(&mut writer, 0, 1);
        bits(&mut writer, 0, 1);
        bits(&mut writer, 0, 1);
        bits(&mut writer, 1, 1);

        bits(&mut writer, ClientMessageKind::Move as u64, 6);
        bits(&mut writer, 3, 4);
        bits(&mut writer, 2, 3);
        bits(&mut writer, 5, 16);
        bits(&mut writer, 0b1_1010, 5);

        bits(&mut writer, ClientMessageKind::VoiceData as u64, 6);
        bits(&mut writer, 3, 16);
        bits(&mut writer, 0x1122_3344_5566_7788, 64);
        bits(&mut writer, 0b101, 3);

        bits(&mut writer, ClientMessageKind::RespondCvarValue as u64, 6);
        bits(&mut writer, u32::MAX as u64, 32);
        bits(&mut writer, 0b1110, 4);
        string(&mut writer, b"sv_cheats");
        string(&mut writer, b"0");

        let options = ParseOptions {
            replay_enabled: true,
            xbox_360: true,
            ..ParseOptions::default()
        };
        let stream =
            ClientMessageStream::parse_with_options(writer.as_slice(), writer.len_bits(), options)
                .unwrap();
        assert_eq!(stream.messages().len(), 4);
        assert!(matches!(
            &stream.messages()[0].data,
            ClientMessageData::ClientInfo {
                custom_files,
                is_replay: Some(true),
                ..
            } if custom_files[0] == Some(0xaabb_ccdd)
        ));
        assert!(matches!(
            stream.messages()[1].data,
            ClientMessageData::Move {
                new_commands: 3,
                backup_commands: 2,
                data: BitSpan { bit_len: 5, .. }
            }
        ));
        assert!(matches!(
            stream.messages()[2].data,
            ClientMessageData::VoiceData {
                xuid: Some(0x1122_3344_5566_7788),
                data: BitSpan { bit_len: 3, .. }
            }
        ));
        assert!(matches!(
            &stream.messages()[3].data,
            ClientMessageData::RespondCvarValue {
                cookie: -1,
                status: -2,
                name,
                value,
            } if name == b"sv_cheats" && value == b"0"
        ));
    }

    #[test]
    fn parses_client_file_checks() {
        let mut writer = BitWriter::new();
        bits(&mut writer, ClientMessageKind::FileCrcCheck as u64, 6);
        bits(&mut writer, 0, 1);
        bits(&mut writer, 1, 2);
        bits(&mut writer, 2, 3);
        bits(&mut writer, 1, 8);
        string(&mut writer, b"props/test.mdl");
        for byte in 0..16 {
            bits(&mut writer, byte, 8);
        }
        for value in 10..16 {
            bits(&mut writer, value, 32);
        }

        bits(&mut writer, ClientMessageKind::FileMd5Check as u64, 6);
        bits(&mut writer, 1, 1);
        bits(&mut writer, 0, 2);
        string(&mut writer, b"CUSTOM");
        bits(&mut writer, 4, 3);
        string(&mut writer, b"test.txt");
        for byte in 16..32 {
            bits(&mut writer, byte, 8);
        }

        let stream = ClientMessageStream::parse_with_options(
            writer.as_slice(),
            writer.len_bits(),
            ParseOptions::default(),
        )
        .unwrap();
        assert!(matches!(
            &stream.messages()[0].data,
            ClientMessageData::FileCrcCheck(check)
                if check.path_id == b"GAME"
                    && check.filename == b"models/props/test.mdl"
                    && check.md5 == Some([0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15])
                    && check.file_fraction == Some(15)
        ));
        assert!(matches!(
            &stream.messages()[1].data,
            ClientMessageData::FileMd5Check(check)
                if check.reserved
                    && check.path_id == b"CUSTOM"
                    && check.filename == b"scripts/test.txt"
                    && check.md5[0] == 16
                    && check.md5[15] == 31
        ));
    }

    #[test]
    fn decodes_protocol_7_demo_widths() {
        let mut writer = BitWriter::new();
        bits(&mut writer, MessageKind::UpdateStringTable as u64, 5);
        bits(&mut writer, 9, 4);
        bits(&mut writer, 0, 1);
        bits(&mut writer, 3, 16);
        bits(&mut writer, 0b101, 3);

        bits(&mut writer, MessageKind::BspDecal as u64, 5);
        bits(&mut writer, 0, 3);
        bits(&mut writer, 12, 9);
        bits(&mut writer, 1, 1);
        bits(&mut writer, 33, 11);
        bits(&mut writer, 0x3ff, 11);
        bits(&mut writer, 1, 1);
        bits(&mut writer, MessageKind::Nop as u64, 5);

        let options = ParseOptions {
            network_protocol: 7,
            required_network_protocol: None,
            message_type_bits: 5,
            ..ParseOptions::default()
        };
        let stream =
            MessageStream::parse_with_options(writer.as_slice(), writer.len_bits(), options)
                .unwrap();
        assert_eq!(stream.messages().len(), 3);
        assert!(matches!(
            stream.messages()[0].data,
            MessageData::UpdateStringTable {
                table_id: 9,
                changed_entries: 1,
                data: BitSpan { bit_len: 3, .. },
            }
        ));
        assert!(matches!(
            stream.messages()[1].data,
            MessageData::BspDecal {
                entity: Some((33, 0x3ff)),
                low_priority: true,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unknown_truncated_and_excessive_messages() {
        let mut writer = BitWriter::new();
        bits(&mut writer, 16, 6);
        assert!(matches!(
            MessageStream::parse_with_options(
                writer.as_slice(),
                writer.len_bits(),
                ParseOptions::default()
            ),
            Err(Error::InvalidMessageType(16))
        ));

        let mut writer = BitWriter::new();
        bits(&mut writer, MessageKind::Print as u64, 6);
        bits(&mut writer, b'x' as u64, 8);
        assert!(MessageStream::parse_with_options(
            writer.as_slice(),
            writer.len_bits(),
            ParseOptions::default()
        )
        .is_err());

        let mut options = ParseOptions::default();
        options.limits.max_messages = 0;
        assert!(MessageStream::parse_with_options(&[], 0, options).is_ok());
        assert!(matches!(
            MessageStream::parse_with_options(&[0], 6, options),
            Err(Error::MessageLimitExceeded(0))
        ));
    }

    #[test]
    fn owns_channel_sequence_and_drop_state() {
        let mut registry = ChannelRegistry::new();
        let channel = registry.create(1, 0, 0);

        assert_eq!(
            registry.advance_outgoing(channel).unwrap(),
            SequenceAdvance {
                previous: 1,
                current: 2,
            }
        );

        let first = registry.preview_incoming(channel, 1, 7, 0, 100).unwrap();
        assert_eq!(
            first,
            PacketDecision {
                incoming_sequence: 1,
                outgoing_ack: 7,
                dropped: 0,
                accepted: true,
                reason: PACKET_ACCEPTED,
            }
        );
        assert_eq!(
            registry.preview_incoming(channel, 1, 7, 0, 100).unwrap(),
            first
        );
        assert_eq!(
            registry.commit_incoming(channel, 1, 7, 0, 100).unwrap(),
            first
        );

        let duplicate = registry.preview_incoming(channel, 1, 9, 0, 100).unwrap();
        assert!(!duplicate.accepted);
        assert_eq!(duplicate.reason, PACKET_DUPLICATE);
        assert_eq!(duplicate.incoming_sequence, 1);
        assert_eq!(duplicate.outgoing_ack, 7);

        let out_of_order = registry.preview_incoming(channel, 0, 9, 0, 100).unwrap();
        assert!(!out_of_order.accepted);
        assert_eq!(out_of_order.reason, PACKET_OUT_OF_ORDER);

        let dropped = registry.preview_incoming(channel, 5, 11, 1, 100).unwrap();
        assert_eq!(dropped.dropped, 2);
        assert!(dropped.accepted);

        let excessive = registry.preview_incoming(channel, 5, 11, 1, 1).unwrap();
        assert!(!excessive.accepted);
        assert_eq!(excessive.reason, PACKET_EXCESSIVE_DROP);
        assert_eq!(
            registry
                .preview_incoming(channel, 2, 8, 0, 100)
                .unwrap()
                .dropped,
            0
        );

        registry.reset(channel, 12, 8, 10).unwrap();
        assert_eq!(
            registry.advance_outgoing(channel).unwrap(),
            SequenceAdvance {
                previous: 12,
                current: 13,
            }
        );
        assert!(registry.remove(channel));
        assert!(matches!(
            registry.advance_outgoing(channel),
            Err(ChannelError::Missing(id)) if id == channel
        ));
    }

    #[test]
    fn parses_and_validates_channel_packet_headers() {
        assert_eq!(short_packet_checksum(b"123456789"), 0xf2d2);

        let sequence = 17i32;
        let outgoing_ack = 11i32;
        let challenge = 0xa1b2_c3d4u32;
        let flags = PACKET_FLAG_CHOKED | PACKET_FLAG_CHALLENGE | 1;
        let encoded =
            encode_packet_header(sequence, outgoing_ack, 0xa5, Some(3), Some(challenge), true);
        assert_eq!(encoded.length, 17);
        assert_eq!(encoded.flags_offset, 8);
        assert_eq!(encoded.checksum_offset, Some(9));
        assert_eq!(encoded.checksum_start, 11);
        assert_eq!(
            encoded.base_flags,
            PACKET_FLAG_CHOKED | PACKET_FLAG_CHALLENGE
        );
        let mut encoded_packet = encoded.bytes[..encoded.length].to_vec();
        encoded_packet.extend_from_slice(b"payload");
        assert_ne!(
            finalize_packet_header(&mut encoded_packet, flags, true).unwrap(),
            0
        );
        assert_eq!(
            parse_packet_header(&encoded_packet, true, false, challenge).reason,
            PACKET_HEADER_ACCEPTED
        );
        assert_eq!(
            finalize_packet_header(&mut [0; 10], flags, true),
            Err(PacketHeaderError::Truncated(10))
        );

        let mut packet = Vec::new();
        packet.extend_from_slice(&sequence.to_le_bytes());
        packet.extend_from_slice(&outgoing_ack.to_le_bytes());
        packet.push(flags);
        packet.extend_from_slice(&0u16.to_le_bytes());
        packet.push(0xa5);
        packet.push(3);
        packet.extend_from_slice(&challenge.to_le_bytes());
        packet.extend_from_slice(b"payload");
        let checksum = short_packet_checksum(&packet[11..]);
        packet[9..11].copy_from_slice(&checksum.to_le_bytes());

        let header = parse_packet_header(&packet, true, false, challenge);
        assert_eq!(
            header,
            PacketHeaderDecision {
                sequence,
                outgoing_ack,
                flags,
                reliable_state: 0xa5,
                choked: 3,
                challenge,
                header_bytes: 17,
                accepted: true,
                reason: PACKET_HEADER_ACCEPTED,
            }
        );

        let mut corrupted = packet.clone();
        *corrupted.last_mut().unwrap() ^= 1;
        assert_eq!(
            parse_packet_header(&corrupted, true, false, challenge).reason,
            PACKET_HEADER_CHECKSUM_MISMATCH
        );
        assert_eq!(
            parse_packet_header(&packet, true, false, challenge.wrapping_add(1)).reason,
            PACKET_HEADER_CHALLENGE_MISMATCH
        );
        assert_eq!(
            parse_packet_header(&packet[..10], true, false, challenge).reason,
            PACKET_HEADER_TRUNCATED
        );

        let mut no_challenge = Vec::new();
        no_challenge.extend_from_slice(&sequence.to_le_bytes());
        no_challenge.extend_from_slice(&outgoing_ack.to_le_bytes());
        no_challenge.push(0);
        no_challenge.push(0);
        assert_eq!(
            parse_packet_header(&no_challenge, false, true, challenge).reason,
            PACKET_HEADER_CHALLENGE_MISSING
        );
    }

    #[test]
    fn owns_network_string_table_contents_and_change_ticks() {
        let mut registry = StringTableRegistry::new();
        assert!(matches!(
            registry.create(b"invalid", 3, 0),
            Err(StringTableError::InvalidCapacity(3))
        ));
        let table = registry.create(b"modelprecache", 4, 1).unwrap();
        assert_eq!(registry.name(table).unwrap(), b"modelprecache");

        let first = registry
            .upsert(table, b"PlayerModel", Some(&[1, 2]))
            .unwrap();
        assert_eq!(first.index, 0);
        assert_eq!(first.entry_count, 1);
        assert!(first.created);
        assert_eq!(registry.string(table, 0).unwrap(), b"PlayerModel");
        assert_eq!(registry.user_data(table, 0).unwrap(), [1, 2]);
        assert_eq!(registry.find(table, b"playermodel").unwrap(), 0);
        assert!(registry.changed_since(table, 0).unwrap());
        assert!(!registry.changed_since(table, 1).unwrap());

        let existing = registry.upsert(table, b"PLAYERMODEL", None).unwrap();
        assert_eq!(existing.index, 0);
        assert!(!existing.created);
        assert!(!existing.user_data_changed);

        registry.set_tick(table, 2).unwrap();
        assert!(!registry.set_user_data(table, 0, &[1, 2]).unwrap().changed);
        let changed = registry.set_user_data(table, 0, &[3, 4]).unwrap();
        assert!(changed.changed);
        assert_eq!(changed.tick_changed, 2);
        assert!(registry.changed_since(table, 1).unwrap());
        assert_eq!(registry.user_data(table, 0).unwrap(), [3, 4]);
        assert!(matches!(
            registry.set_tick(table, 1),
            Err(StringTableError::TickRegression {
                current: 2,
                requested: 1,
            })
        ));

        for value in [b"one".as_slice(), b"two", b"three"] {
            registry.upsert(table, value, None).unwrap();
        }
        assert!(matches!(
            registry.upsert(table, b"four", None),
            Err(StringTableError::TableFull(4))
        ));
        registry.clear(table).unwrap();
        assert_eq!(registry.entry_count(table).unwrap(), 0);
        assert!(!registry.changed_since(table, 0).unwrap());
        assert!(registry.remove(table));
    }

    #[test]
    fn encodes_network_string_table_entries_for_containers() {
        let mut registry = StringTableRegistry::new();
        let table = registry.create(b"downloadables", 8, 1).unwrap();
        registry.upsert(table, b"ab", Some(&[0x41, 0x42])).unwrap();
        registry.upsert(table, b"c", None).unwrap();

        let (bytes, bits) = registry.encode_entries(table).unwrap();
        // 16-bit count, then "ab\0" + set flag + 16-bit length + 2 bytes, then
        // "c\0" + clear flag.
        assert_eq!(bits, 16 + (3 * 8 + 1 + 16 + 2 * 8) + (2 * 8 + 1));
        let mut reader = BitReader::new(&bytes);
        assert_eq!(reader.read_bits(16).unwrap(), 2);
        let mut name = Vec::new();
        loop {
            let byte = reader.read_bits(8).unwrap() as u8;
            if byte == 0 {
                break;
            }
            name.push(byte);
        }
        assert_eq!(name, b"ab");
        assert!(reader.read_bool().unwrap());
        assert_eq!(reader.read_bits(16).unwrap(), 2);
        assert_eq!(reader.read_bits(8).unwrap(), 0x41);
        assert_eq!(reader.read_bits(8).unwrap(), 0x42);
        assert_eq!(reader.read_bits(8).unwrap(), u64::from(b'c'));
        assert_eq!(reader.read_bits(8).unwrap(), 0);
        assert!(!reader.read_bool().unwrap());

        let empty = registry.create(b"empty", 4, 1).unwrap();
        assert_eq!(registry.encode_entries(empty).unwrap().1, 16);
        assert!(matches!(
            registry.encode_entries(table + 1000),
            Err(StringTableError::MissingTable(_))
        ));
    }

    #[test]
    fn decodes_the_string_table_container_it_encodes() {
        let mut registry = StringTableRegistry::new();
        let first = registry.create(b"downloadables", 8, 1).unwrap();
        registry.upsert(first, b"ab", Some(&[0x41, 0x42])).unwrap();
        registry.upsert(first, b"c", None).unwrap();
        let second = registry.create(b"modelprecache", 8, 1).unwrap();
        registry.upsert(second, b"models/gman.mdl", None).unwrap();

        // Framed the way the engine's container writes it: a table count, then
        // each table's name, its entries, and the client-side flag.
        let mut writer = BitWriter::new();
        writer.write_bits(2, 8).unwrap();
        for table in [first, second] {
            for byte in registry.name(table).unwrap() {
                writer.write_bits(u64::from(*byte), 8).unwrap();
            }
            writer.write_bits(0, 8).unwrap();
            let (bytes, bits) = registry.encode_entries(table).unwrap();
            let mut reader = BitReader::new(&bytes);
            for _ in 0..bits {
                writer.write_bool(reader.read_bool().unwrap()).unwrap();
            }
            writer.write_bool(false).unwrap();
        }

        let sections = decode_string_table_container(writer.as_slice()).unwrap();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, b"downloadables");
        assert_eq!(
            sections[0].entries,
            vec![
                (b"ab".to_vec(), vec![0x41, 0x42]),
                (b"c".to_vec(), Vec::new()),
            ]
        );
        assert_eq!(sections[0].client_side, None);
        assert_eq!(sections[1].name, b"modelprecache");
        assert_eq!(
            sections[1].entries,
            vec![(b"models/gman.mdl".to_vec(), Vec::new())]
        );

        let truncated = &writer.as_slice()[..writer.as_slice().len() - 1];
        assert!(matches!(
            decode_string_table_container(truncated),
            Err(StringTableError::Truncated)
        ));
    }

    #[test]
    fn restores_network_string_table_user_data_history() {
        let mut registry = StringTableRegistry::new();
        let table = registry.create(b"instancebaseline", 4, 1).unwrap();
        registry.enable_history(table).unwrap();
        registry.upsert(table, b"baseline", Some(b"one")).unwrap();
        registry.set_tick(table, 2).unwrap();
        let staged = registry.set_user_data(table, 0, b"two").unwrap();
        assert!(!staged.changed);
        assert_eq!(staged.tick_changed, 1);
        assert!(!registry.changed_since(table, 0).unwrap());
        registry.set_tick(table, 3).unwrap();
        registry.set_user_data(table, 0, b"three").unwrap();

        assert_eq!(registry.restore_tick(table, 1).unwrap(), 1);
        assert_eq!(registry.user_data(table, 0).unwrap(), b"one");
        assert!(registry.changed_since(table, 0).unwrap());
        assert_eq!(registry.restore_tick(table, 2).unwrap(), 2);
        assert_eq!(registry.user_data(table, 0).unwrap(), b"two");
        assert_eq!(registry.restore_tick(table, 0).unwrap(), 0);
        assert_eq!(registry.user_data(table, 0).unwrap(), b"");
    }

    #[test]
    fn owns_and_validates_server_datatable_metadata() {
        let mut registry = DataTableRegistry::new();
        let base = registry.register_table(b"DT_BaseEntity", 1).unwrap();
        registry
            .register_property(
                base.table_id,
                DataTablePropertyDescriptor {
                    name: b"m_iTeamNum".to_vec(),
                    reference_name: Vec::new(),
                    property_type: 0,
                    flags: 1,
                    bit_count: 8,
                    elements: 1,
                    low_value: 0.0,
                    high_value: 255.0,
                },
            )
            .unwrap();
        let derived = registry.register_table(b"DT_TestEntity", 1).unwrap();
        registry
            .register_property(
                derived.table_id,
                DataTablePropertyDescriptor {
                    name: b"baseclass".to_vec(),
                    reference_name: b"DT_BaseEntity".to_vec(),
                    property_type: SEND_PROP_DATA_TABLE,
                    flags: 1 << 12,
                    bit_count: 0,
                    elements: 1,
                    low_value: 0.0,
                    high_value: 0.0,
                },
            )
            .unwrap();
        assert_eq!(
            registry
                .register_class(b"CTestEntity", derived.table_id)
                .unwrap(),
            0
        );

        let crc = match registry.finalize(0) {
            Err(DataTableError::CompatibilityCrcMismatch { actual, .. }) => actual,
            result => panic!("expected compatibility CRC mismatch, got {result:?}"),
        };
        let summary = registry.finalize(crc).unwrap();
        assert_eq!(summary.table_count, 2);
        assert_eq!(summary.property_count, 2);
        assert_eq!(summary.class_count, 1);
        assert_eq!(summary.compatibility_crc, crc);
        assert_eq!(registry.class_id(b"ctestentity").unwrap(), 0);
        assert!(matches!(
            registry.register_table(b"DT_TooLate", 0),
            Err(DataTableError::Finalized)
        ));
    }

    #[test]
    fn rejects_incomplete_or_dangling_datatable_schemas() {
        let mut incomplete = DataTableRegistry::new();
        let table = incomplete.register_table(b"DT_Incomplete", 1).unwrap();
        incomplete
            .register_class(b"CIncomplete", table.table_id)
            .unwrap();
        assert!(matches!(
            incomplete.finalize(0),
            Err(DataTableError::PropertyCountMismatch { .. })
        ));

        let mut dangling = DataTableRegistry::new();
        let table = dangling.register_table(b"DT_Dangling", 1).unwrap();
        dangling
            .register_property(
                table.table_id,
                DataTablePropertyDescriptor {
                    name: b"baseclass".to_vec(),
                    reference_name: b"DT_Missing".to_vec(),
                    property_type: SEND_PROP_DATA_TABLE,
                    flags: 0,
                    bit_count: 0,
                    elements: 1,
                    low_value: 0.0,
                    high_value: 0.0,
                },
            )
            .unwrap();
        dangling
            .register_class(b"CDangling", table.table_id)
            .unwrap();
        assert!(matches!(
            dangling.finalize(0),
            Err(DataTableError::UnknownReferencedTable(name)) if name == b"DT_Missing"
        ));
    }

    #[test]
    fn owns_snapshot_entity_order_and_explicit_deletes() {
        let mut registry = SnapshotRegistry::new();
        assert!(registry.queue_explicit_delete(17).unwrap());
        assert!(!registry.queue_explicit_delete(17).unwrap());
        assert!(registry.queue_explicit_delete(29).unwrap());

        let entities = [
            SnapshotEntity {
                entity_index: 0,
                serial_number: 7,
                class_id: 2,
            },
            SnapshotEntity {
                entity_index: 11,
                serial_number: 19,
                class_id: 5,
            },
        ];
        let (snapshot, summary) = registry.create(123, 64, &entities).unwrap();
        assert_eq!(
            summary,
            SnapshotSummary {
                tick: 123,
                max_entities: 64,
                valid_entity_count: 2,
                explicit_delete_count: 2,
            }
        );
        assert_eq!(registry.entity(snapshot, 1).unwrap(), entities[1]);
        assert_eq!(registry.explicit_delete(snapshot, 0).unwrap(), 17);
        assert_eq!(registry.explicit_delete(snapshot, 1).unwrap(), 29);

        let (next, next_summary) = registry.create(124, 64, &entities[..1]).unwrap();
        assert_eq!(next_summary.explicit_delete_count, 0);
        assert!(registry.remove(snapshot));
        assert!(registry.remove(next));
        assert!(matches!(
            registry.summary(snapshot),
            Err(SnapshotError::Missing(id)) if id == snapshot
        ));
    }

    #[test]
    fn rejects_invalid_snapshot_entities_without_consuming_deletes() {
        let mut registry = SnapshotRegistry::new();
        registry.queue_explicit_delete(9).unwrap();
        let reversed = [
            SnapshotEntity {
                entity_index: 2,
                serial_number: 1,
                class_id: 0,
            },
            SnapshotEntity {
                entity_index: 1,
                serial_number: 2,
                class_id: 0,
            },
        ];
        assert!(matches!(
            registry.create(1, 16, &reversed),
            Err(SnapshotError::EntityOrder {
                previous: 2,
                current: 1,
            })
        ));
        assert!(matches!(
            registry.create(
                1,
                16,
                &[SnapshotEntity {
                    entity_index: 16,
                    serial_number: 1,
                    class_id: 0,
                }]
            ),
            Err(SnapshotError::InvalidEntityIndex(16))
        ));
        let (_, summary) = registry.create(2, 16, &[]).unwrap();
        assert_eq!(summary.explicit_delete_count, 1);
    }

    #[test]
    fn classifies_snapshot_delta_in_wire_order() {
        let mut registry = SnapshotRegistry::new();
        let entity = |entity_index, serial_number, class_id| SnapshotEntity {
            entity_index,
            serial_number,
            class_id,
        };

        // Index 1 leaves, 2 is unchanged in identity, 3 is recreated under a new
        // serial, 4 keeps its serial but changes class, and 9 is brand new.
        let (from, _) = registry
            .create(
                40,
                64,
                &[
                    entity(1, 3, 0),
                    entity(2, 4, 1),
                    entity(3, 5, 2),
                    entity(4, 6, 3),
                ],
            )
            .unwrap();
        let (to, _) = registry
            .create(
                41,
                64,
                &[
                    entity(2, 4, 1),
                    entity(3, 7, 2),
                    entity(4, 6, 8),
                    entity(9, 11, 4),
                ],
            )
            .unwrap();

        assert_eq!(
            registry
                .delta(
                    Some(SnapshotDeltaSide::whole(from)),
                    SnapshotDeltaSide::whole(to)
                )
                .unwrap(),
            vec![
                SnapshotDeltaEntry {
                    entity_index: 1,
                    kind: SnapshotDeltaKind::LeavePvs,
                    class_id: 0,
                    serial_number: 3,
                    recreated: false,
                },
                SnapshotDeltaEntry {
                    entity_index: 2,
                    kind: SnapshotDeltaKind::DeltaCandidate,
                    class_id: 1,
                    serial_number: 4,
                    recreated: false,
                },
                SnapshotDeltaEntry {
                    entity_index: 3,
                    kind: SnapshotDeltaKind::EnterPvs,
                    class_id: 2,
                    serial_number: 7,
                    recreated: true,
                },
                SnapshotDeltaEntry {
                    entity_index: 4,
                    kind: SnapshotDeltaKind::EnterPvs,
                    class_id: 8,
                    serial_number: 6,
                    recreated: true,
                },
                SnapshotDeltaEntry {
                    entity_index: 9,
                    kind: SnapshotDeltaKind::EnterPvs,
                    class_id: 4,
                    serial_number: 11,
                    recreated: false,
                },
            ]
        );
    }

    #[test]
    fn treats_a_full_update_as_every_entity_entering() {
        let mut registry = SnapshotRegistry::new();
        let entities = [
            SnapshotEntity {
                entity_index: 0,
                serial_number: 1,
                class_id: 5,
            },
            SnapshotEntity {
                entity_index: 30,
                serial_number: 2,
                class_id: 6,
            },
        ];
        let (to, _) = registry.create(7, 64, &entities).unwrap();

        let delta = registry.delta(None, SnapshotDeltaSide::whole(to)).unwrap();
        assert_eq!(delta.len(), entities.len());
        assert!(delta
            .iter()
            .all(|entry| entry.kind == SnapshotDeltaKind::EnterPvs && !entry.recreated));
        assert_eq!(delta[1].entity_index, 30);

        // Delta against itself must report no creations, deletions, or recreates.
        let identical = registry
            .delta(
                Some(SnapshotDeltaSide::whole(to)),
                SnapshotDeltaSide::whole(to),
            )
            .unwrap();
        assert!(identical
            .iter()
            .all(|entry| entry.kind == SnapshotDeltaKind::DeltaCandidate));
        assert_eq!(identical.len(), entities.len());

        assert!(matches!(
            registry.delta(
                Some(SnapshotDeltaSide::whole(to + 1000)),
                SnapshotDeltaSide::whole(to)
            ),
            Err(SnapshotError::Missing(_))
        ));
    }

    #[test]
    fn narrows_snapshot_delta_to_the_visible_set() {
        let mut registry = SnapshotRegistry::new();
        let entity = |entity_index, serial_number, class_id| SnapshotEntity {
            entity_index,
            serial_number,
            class_id,
        };
        let entities = [
            entity(1, 1, 0),
            entity(2, 2, 1),
            entity(3, 3, 2),
            entity(4, 4, 3),
        ];
        let (from, _) = registry.create(10, 64, &entities).unwrap();
        let (to, _) = registry.create(11, 64, &entities).unwrap();

        // The snapshots are identical, so every difference here comes from the
        // visibility sets alone: 1 drops out, 2 stays, and 4 becomes visible.
        let delta = registry
            .delta(
                Some(SnapshotDeltaSide {
                    snapshot_id: from,
                    transmit: Some(&[1, 2]),
                }),
                SnapshotDeltaSide {
                    snapshot_id: to,
                    transmit: Some(&[2, 4]),
                },
            )
            .unwrap();
        assert_eq!(
            delta
                .iter()
                .map(|entry| (entry.entity_index, entry.kind))
                .collect::<Vec<_>>(),
            vec![
                (1, SnapshotDeltaKind::LeavePvs),
                (2, SnapshotDeltaKind::DeltaCandidate),
                (4, SnapshotDeltaKind::EnterPvs),
            ]
        );

        // An index the newer snapshot has no entity for cannot be created.
        assert!(matches!(
            registry.delta(
                None,
                SnapshotDeltaSide {
                    snapshot_id: to,
                    transmit: Some(&[2, 40]),
                }
            ),
            Err(SnapshotError::MissingEntity(40))
        ));

        // An index the older snapshot has no entity for has nothing to delta
        // against, so it must be recreated rather than silently preserved.
        let (sparse, _) = registry.create(9, 64, &[entity(2, 2, 1)]).unwrap();
        let recreated = registry
            .delta(
                Some(SnapshotDeltaSide {
                    snapshot_id: sparse,
                    transmit: Some(&[2, 3]),
                }),
                SnapshotDeltaSide {
                    snapshot_id: to,
                    transmit: Some(&[2, 3]),
                },
            )
            .unwrap();
        assert_eq!(recreated[0].kind, SnapshotDeltaKind::DeltaCandidate);
        assert_eq!(recreated[1].kind, SnapshotDeltaKind::EnterPvs);
        assert!(recreated[1].recreated);

        // Out-of-order and duplicated visibility sets are rejected outright.
        assert!(matches!(
            registry.delta(
                None,
                SnapshotDeltaSide {
                    snapshot_id: to,
                    transmit: Some(&[3, 2]),
                }
            ),
            Err(SnapshotError::EntityOrder {
                previous: 3,
                current: 2
            })
        ));
        assert!(matches!(
            registry.delta(
                None,
                SnapshotDeltaSide {
                    snapshot_id: to,
                    transmit: Some(&[2, 2]),
                }
            ),
            Err(SnapshotError::DuplicateEntity(2))
        ));
    }

    #[test]
    fn encodes_delta_headers_like_the_native_writer() {
        // Reads `count` bits back out in wire order.
        let decode = |bytes: &[u8], count: u32| {
            let mut reader = BitReader::new(bytes);
            (0..count)
                .map(|_| reader.read_bits(1).unwrap())
                .collect::<Vec<_>>()
        };

        // A gap under 16 uses the two-bit selector 0 and a four-bit payload,
        // then a cleared removal bit and the creation bit.
        let (bytes, count) = DeltaHeader {
            entity_index: 6,
            header_base: 1,
            leave_pvs: false,
            delete_entity: false,
            enter_pvs: true,
        }
        .encode()
        .unwrap();
        assert_eq!(count, 8);
        assert_eq!(
            decode(&bytes, count),
            vec![0, 0, /* gap 4 == 0b0100 */ 0, 0, 1, 0, /* flags */ 0, 1]
        );

        // A removal writes the removal bit and then the delete bit.
        let (bytes, count) = DeltaHeader {
            entity_index: 0,
            header_base: -1,
            leave_pvs: true,
            delete_entity: true,
            enter_pvs: false,
        }
        .encode()
        .unwrap();
        assert_eq!(count, 8);
        assert_eq!(decode(&bytes, count), vec![0, 0, 0, 0, 0, 0, 1, 1]);

        // Each wider gap moves up one selector and payload width. The entity
        // index bound keeps gaps under 4096, so the widest selector the native
        // table defines is unreachable for entity headers.
        for (gap, selector, payload_bits) in [(16u32, 1u32, 8u32), (256, 2, 12)] {
            let (bytes, count) = DeltaHeader {
                entity_index: gap + 1,
                header_base: 0,
                leave_pvs: false,
                delete_entity: false,
                enter_pvs: false,
            }
            .encode()
            .unwrap();
            assert_eq!(count, 2 + payload_bits + 2);
            let bits = decode(&bytes, count);
            assert_eq!(bits[0] | (bits[1] << 1), u64::from(selector));
            let payload = bits[2..2 + payload_bits as usize]
                .iter()
                .enumerate()
                .fold(0u64, |value, (index, bit)| value | (bit << index));
            assert_eq!(payload, u64::from(gap));
        }
        assert!(count_bits_fit());

        assert!(matches!(
            DeltaHeader {
                entity_index: 4,
                header_base: 4,
                leave_pvs: false,
                delete_entity: false,
                enter_pvs: false,
            }
            .encode(),
            Err(DeltaHeaderError::NonAscendingIndex {
                header_base: 4,
                entity_index: 4
            })
        ));
        assert!(matches!(
            DeltaHeader {
                entity_index: 4,
                header_base: -1,
                leave_pvs: true,
                delete_entity: false,
                enter_pvs: true,
            }
            .encode(),
            Err(DeltaHeaderError::ConflictingFlags)
        ));
    }

    // The widest encoding has to stay within the advertised bound, because
    // callers size fixed buffers from it.
    fn count_bits_fit() -> bool {
        let (_, count) = DeltaHeader {
            entity_index: MAX_SNAPSHOT_ENTITIES as u32 - 1,
            header_base: -1,
            leave_pvs: false,
            delete_entity: false,
            enter_pvs: false,
        }
        .encode()
        .unwrap();
        count <= MAX_DELTA_HEADER_BITS
    }

    #[test]
    fn wraps_outgoing_sequence_like_the_native_channel() {
        let mut registry = ChannelRegistry::new();
        let channel = registry.create(i32::MAX, 0, 0);
        assert_eq!(
            registry.advance_outgoing(channel).unwrap(),
            SequenceAdvance {
                previous: i32::MAX,
                current: i32::MIN,
            }
        );
    }

    /// A payload with a recognisable pattern, so a piece landing at the wrong
    /// offset shows up as wrong bytes rather than merely the wrong length.
    fn patterned(length: usize) -> Vec<u8> {
        (0..length).map(|index| (index % 251) as u8).collect()
    }

    #[test]
    fn splits_and_reassembles_a_payload() {
        let payload = patterned(4000);
        let pieces = split_packet(&payload, 7, 1260).expect("split");
        assert_eq!(pieces.len(), 4);

        // Every piece but the last is exactly the split size, which is what
        // lets a receiver compute offsets without a length per piece.
        for piece in &pieces[..3] {
            assert_eq!(piece.len(), 1260);
        }
        assert_eq!(pieces[3].len(), SPLIT_PACKET_HEADER_BYTES + 4000 - 3 * 1248);

        let header = SplitPacketHeader::decode(&pieces[2]).expect("decode");
        assert_eq!(
            header,
            SplitPacketHeader {
                sequence: 7,
                packet_number: 2,
                packet_count: 4,
                split_size: 1248,
            }
        );
        assert_eq!(header.encode(), pieces[2][..SPLIT_PACKET_HEADER_BYTES]);

        let mut reassembler = SplitPacketReassembler::new();
        assert!(!reassembler.is_assembling());
        for piece in &pieces[..3] {
            assert_eq!(reassembler.accept(piece), Ok(None));
            assert!(reassembler.is_assembling());
        }
        assert_eq!(reassembler.accept(&pieces[3]), Ok(Some(payload)));
        // A completed message leaves nothing behind for the next one.
        assert!(!reassembler.is_assembling());
    }

    #[test]
    fn reassembles_pieces_that_arrive_out_of_order_or_twice() {
        let payload = patterned(3000);
        let pieces = split_packet(&payload, -3, 1260).expect("split");
        assert_eq!(pieces.len(), 3);

        let mut reassembler = SplitPacketReassembler::new();
        // The last piece first, which is also what fixes the total size, then
        // a repeat of it: a duplicate must not be counted toward completion.
        assert_eq!(reassembler.accept(&pieces[2]), Ok(None));
        assert_eq!(reassembler.accept(&pieces[2]), Ok(None));
        assert_eq!(reassembler.accept(&pieces[1]), Ok(None));
        assert_eq!(reassembler.accept(&pieces[0]), Ok(Some(payload)));
    }

    #[test]
    fn a_new_sequence_abandons_the_message_in_progress() {
        let first = patterned(2000);
        let second = patterned(1500);
        let first_pieces = split_packet(&first, 1, 1260).expect("split");
        let second_pieces = split_packet(&second, 2, 1260).expect("split");

        let mut reassembler = SplitPacketReassembler::new();
        assert_eq!(reassembler.accept(&first_pieces[0]), Ok(None));
        // The sender moved on, so the half-received message can never
        // complete and its pieces must not contaminate the new one.
        assert_eq!(reassembler.accept(&second_pieces[0]), Ok(None));
        assert_eq!(reassembler.accept(&second_pieces[1]), Ok(Some(second)));
    }

    #[test]
    fn refuses_pieces_that_cannot_be_placed() {
        let payload = patterned(3000);
        let pieces = split_packet(&payload, 5, 1260).expect("split");

        assert_eq!(
            SplitPacketHeader::decode(&pieces[0][..8]),
            Err(SplitPacketError::Truncated(8))
        );

        let mut not_split = pieces[0].clone();
        not_split[0..4].copy_from_slice(&(-1i32).to_le_bytes());
        assert_eq!(
            SplitPacketHeader::decode(&not_split),
            Err(SplitPacketError::NotSplit(-1))
        );

        // Below the X.25 floor the sender could not have cut here at all.
        let mut tiny = pieces[0].clone();
        tiny[10..12].copy_from_slice(&64u16.to_le_bytes());
        assert_eq!(
            SplitPacketHeader::decode(&tiny),
            Err(SplitPacketError::SplitSizeOutOfRange(64))
        );

        // Position three of three pieces does not exist.
        let mut past_end = pieces[0].clone();
        past_end[8..10].copy_from_slice(&((3u16 << 8) | 3).to_le_bytes());
        assert_eq!(
            SplitPacketHeader::decode(&past_end),
            Err(SplitPacketError::PositionOutOfRange {
                number: 3,
                count: 3
            })
        );

        // A short middle piece would leave the gap to the next offset holding
        // whatever the buffer already had.
        let mut short = pieces[0].clone();
        short.truncate(SPLIT_PACKET_HEADER_BYTES + 16);
        let mut reassembler = SplitPacketReassembler::new();
        assert_eq!(
            reassembler.accept(&short),
            Err(SplitPacketError::ShortPiece {
                number: 0,
                expected: 1248,
                found: 16
            })
        );

        // Offsets come from the split size, so a piece that disagrees with
        // the one the message started on cannot be placed.
        assert_eq!(reassembler.accept(&pieces[0]), Ok(None));
        let mut disagrees = pieces[1].clone();
        disagrees[10..12].copy_from_slice(&1000u16.to_le_bytes());
        disagrees.truncate(SPLIT_PACKET_HEADER_BYTES + 1000);
        assert_eq!(
            reassembler.accept(&disagrees),
            Err(SplitPacketError::InconsistentSplitSize {
                expected: 1248,
                found: 1000
            })
        );
    }

    #[test]
    fn refuses_payloads_that_cannot_be_cut() {
        // A datagram is only cut up because it was too big to route whole, so
        // an empty one is a caller mistake rather than zero pieces.
        assert_eq!(
            split_packet(&[], 1, 1260),
            Err(SplitPacketError::EmptyPayload)
        );
        assert_eq!(
            split_packet(&[0u8; 16], 1, 128),
            Err(SplitPacketError::SplitSizeOutOfRange(116))
        );
        assert_eq!(
            split_packet(&[0u8; 16], 1, 9000),
            Err(SplitPacketError::SplitSizeOutOfRange(8988))
        );
        assert_eq!(
            split_packet(&vec![0u8; MAX_REASSEMBLED_BYTES + 1], 1, 1260),
            Err(SplitPacketError::TooLarge(MAX_REASSEMBLED_BYTES + 1))
        );
        // The count travels in a single byte, so at the smallest split size a
        // full message needs more pieces than the wire can describe.
        let payload = vec![0u8; 255 * MIN_SPLIT_PAYLOAD_BYTES + 1];
        assert_eq!(
            split_packet(&payload, 1, 576),
            Err(SplitPacketError::Unsplittable {
                payload: payload.len(),
                split_size: MIN_SPLIT_PAYLOAD_BYTES
            })
        );
    }

    #[test]
    fn split_sizes_match_the_native_constants() {
        assert_eq!(SPLIT_PACKET_HEADER_BYTES, 12);
        assert_eq!(MAX_SPLIT_PAYLOAD_BYTES, 1248);
        assert_eq!(MIN_SPLIT_PAYLOAD_BYTES, 564);
        assert_eq!(MAX_SPLIT_COUNT, 510);
        assert_eq!(SPLIT_PACKET_FLAG, -2);
    }

    #[test]
    fn describes_every_split_failure() {
        for error in [
            SplitPacketError::Truncated(8),
            SplitPacketError::NotSplit(-1),
            SplitPacketError::SplitSizeOutOfRange(64),
            SplitPacketError::PositionOutOfRange {
                number: 3,
                count: 3,
            },
            SplitPacketError::TooManyPieces(900),
            SplitPacketError::InconsistentSplitSize {
                expected: 1248,
                found: 1000,
            },
            SplitPacketError::ShortPiece {
                number: 0,
                expected: 1248,
                found: 16,
            },
            SplitPacketError::TooLarge(MAX_REASSEMBLED_BYTES + 1),
            SplitPacketError::Unsplittable {
                payload: 1,
                split_size: 564,
            },
            SplitPacketError::EmptyPayload,
        ] {
            assert!(!error.to_string().is_empty());
        }
    }
}
