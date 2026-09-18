//! Bounded parser for Source demo protocol 3 / network protocol 25 files.

use source_binary::Reader;
use std::fmt;
use std::ops::Range;

pub const DEMO_STAMP: &[u8; 8] = b"HL2DEMO\0";
pub const DEMO_PROTOCOL: i32 = 3;
pub const NETWORK_PROTOCOL: i32 = 25;
pub const HEADER_SIZE: usize = 1072;
const CMD_INFO_SIZE: usize = 76;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_file_size: usize,
    pub max_commands: usize,
    pub max_payload_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_file_size: 4 * 1024 * 1024 * 1024usize,
            max_commands: 10_000_000,
            max_payload_size: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ParseOptions {
    pub limits: Limits,
    pub required_demo_protocol: Option<i32>,
    pub required_network_protocol: Option<i32>,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            limits: Limits::default(),
            required_demo_protocol: Some(DEMO_PROTOCOL),
            required_network_protocol: Some(NETWORK_PROTOCOL),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header<'a> {
    pub demo_protocol: i32,
    pub network_protocol: i32,
    pub server_name: &'a str,
    pub client_name: &'a str,
    pub map_name: &'a str,
    pub game_directory: &'a str,
    pub playback_time: f32,
    pub playback_ticks: i32,
    pub playback_frames: i32,
    pub signon_length: i32,
}

/// Fixed width of each name field in the demo header.
const HEADER_NAME_SIZE: usize = 260;

impl Header<'_> {
    /// Encodes the header into the exact 1072 bytes a demo file starts with.
    ///
    /// Each name occupies a fixed 260-byte field that has to stay
    /// null-terminated, so a name that would fill the field is rejected rather
    /// than truncated into something a reader would misinterpret.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if !self.playback_time.is_finite() || self.playback_time < 0.0 {
            return Err(Error::InvalidHeaderValue("playback time"));
        }
        for (value, field) in [
            (self.playback_ticks, "playback ticks"),
            (self.playback_frames, "playback frames"),
            (self.signon_length, "signon length"),
        ] {
            if value < 0 {
                return Err(Error::InvalidHeaderValue(field));
            }
        }

        let mut bytes = Vec::with_capacity(HEADER_SIZE);
        bytes.extend_from_slice(DEMO_STAMP);
        bytes.extend_from_slice(&self.demo_protocol.to_le_bytes());
        bytes.extend_from_slice(&self.network_protocol.to_le_bytes());
        for (name, field) in [
            (self.server_name, "server name"),
            (self.client_name, "client name"),
            (self.map_name, "map name"),
            (self.game_directory, "game directory"),
        ] {
            if name.len() >= HEADER_NAME_SIZE || name.as_bytes().contains(&0) {
                return Err(Error::InvalidHeaderValue(field));
            }
            bytes.extend_from_slice(name.as_bytes());
            bytes.resize(bytes.len() + HEADER_NAME_SIZE - name.len(), 0);
        }
        bytes.extend_from_slice(&self.playback_time.to_le_bytes());
        bytes.extend_from_slice(&self.playback_ticks.to_le_bytes());
        bytes.extend_from_slice(&self.playback_frames.to_le_bytes());
        bytes.extend_from_slice(&self.signon_length.to_le_bytes());
        debug_assert_eq!(bytes.len(), HEADER_SIZE);
        Ok(bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommandInfo {
    pub flags: u32,
    pub view_origin: [f32; 3],
    pub view_angles: [f32; 3],
    pub local_view_angles: [f32; 3],
    pub view_origin2: [f32; 3],
    pub view_angles2: [f32; 3],
    pub local_view_angles2: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Packet<'a> {
    pub info: CommandInfo,
    pub incoming_sequence: i32,
    pub outgoing_acknowledged: i32,
    pub data: &'a [u8],
}

#[derive(Debug, Clone, PartialEq)]
pub enum CommandData<'a> {
    Signon(Packet<'a>),
    Packet(Packet<'a>),
    SyncTick,
    ConsoleCommand(&'a str),
    UserCommand {
        outgoing_sequence: i32,
        data: &'a [u8],
    },
    DataTables(&'a [u8]),
    Stop,
    StringTables(&'a [u8]),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Command<'a> {
    pub tick: i32,
    pub file_range: Range<usize>,
    pub data: CommandData<'a>,
}

#[derive(Debug, Clone)]
pub struct Demo<'a> {
    bytes: &'a [u8],
    header: Header<'a>,
    commands: Vec<Command<'a>>,
}

impl<'a> Demo<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self> {
        Self::parse_with_options(bytes, ParseOptions::default())
    }

    pub fn parse_with_options(bytes: &'a [u8], options: ParseOptions) -> Result<Self> {
        if bytes.len() > options.limits.max_file_size {
            return Err(Error::FileTooLarge {
                size: bytes.len(),
                limit: options.limits.max_file_size,
            });
        }
        let mut reader = Reader::new(bytes);
        let stamp: [u8; 8] = reader.take(8)?.try_into().expect("eight-byte slice");
        if &stamp != DEMO_STAMP {
            return Err(Error::InvalidStamp(stamp));
        }
        let demo_protocol = reader.read_i32_le()?;
        if options
            .required_demo_protocol
            .is_some_and(|required| demo_protocol != required)
        {
            return Err(Error::UnsupportedDemoProtocol(demo_protocol));
        }
        let network_protocol = reader.read_i32_le()?;
        if options
            .required_network_protocol
            .is_some_and(|required| network_protocol != required)
        {
            return Err(Error::UnsupportedNetworkProtocol(network_protocol));
        }
        let server_name = fixed_string(reader.take(260)?, "server name")?;
        let client_name = fixed_string(reader.take(260)?, "client name")?;
        let map_name = fixed_string(reader.take(260)?, "map name")?;
        let game_directory = fixed_string(reader.take(260)?, "game directory")?;
        let playback_time = reader.read_f32_le()?;
        if !playback_time.is_finite() || playback_time < 0.0 {
            return Err(Error::InvalidHeaderValue("playback time"));
        }
        let playback_ticks = nonnegative("playback ticks", reader.read_i32_le()?)?;
        let playback_frames = nonnegative("playback frames", reader.read_i32_le()?)?;
        let signon_length = nonnegative("signon length", reader.read_i32_le()?)?;
        if signon_length as usize > bytes.len().saturating_sub(HEADER_SIZE) {
            return Err(Error::InvalidHeaderValue("signon length"));
        }
        let header = Header {
            demo_protocol,
            network_protocol,
            server_name,
            client_name,
            map_name,
            game_directory,
            playback_time,
            playback_ticks,
            playback_frames,
            signon_length,
        };

        let mut commands = Vec::new();
        let mut saw_stop = false;
        while reader.position() < bytes.len() {
            if commands.len() >= options.limits.max_commands {
                return Err(Error::CommandLimitExceeded(options.limits.max_commands));
            }
            let start = reader.position();
            let command = reader.read_u8()?;
            let tick = reader.read_i32_le()?;
            let data = match command {
                1 => CommandData::Signon(read_packet(&mut reader, options.limits)?),
                2 => CommandData::Packet(read_packet(&mut reader, options.limits)?),
                3 => CommandData::SyncTick,
                4 => {
                    let data = read_payload(&mut reader, options.limits)?;
                    let command = nul_string(data, "console command")?;
                    CommandData::ConsoleCommand(command)
                }
                5 => {
                    let outgoing_sequence = reader.read_i32_le()?;
                    let data = read_payload(&mut reader, options.limits)?;
                    CommandData::UserCommand {
                        outgoing_sequence,
                        data,
                    }
                }
                6 => CommandData::DataTables(read_payload(&mut reader, options.limits)?),
                7 => {
                    saw_stop = true;
                    CommandData::Stop
                }
                8 => CommandData::StringTables(read_payload(&mut reader, options.limits)?),
                value => return Err(Error::InvalidCommand(value)),
            };
            commands.push(Command {
                tick,
                file_range: start..reader.position(),
                data,
            });
            if saw_stop {
                break;
            }
        }
        if !saw_stop {
            return Err(Error::MissingStopCommand);
        }
        if reader.position() != bytes.len() {
            return Err(Error::TrailingData(bytes.len() - reader.position()));
        }
        Ok(Self {
            bytes,
            header,
            commands,
        })
    }

    pub fn header(&self) -> &Header<'a> {
        &self.header
    }

    pub fn commands(&self) -> &[Command<'a>] {
        &self.commands
    }

    pub fn original_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

#[derive(Debug)]
pub enum Error {
    Binary(source_binary::Error),
    FileTooLarge { size: usize, limit: usize },
    InvalidStamp([u8; 8]),
    UnsupportedDemoProtocol(i32),
    UnsupportedNetworkProtocol(i32),
    UnterminatedString(&'static str),
    InvalidUtf8(&'static str),
    InvalidHeaderValue(&'static str),
    InvalidCommand(u8),
    CommandLimitExceeded(usize),
    PayloadTooLarge { size: usize, limit: usize },
    InvalidPayloadLength(i32),
    InvalidCommandInfoFloat,
    ConsoleCommandNotTerminated,
    ConsoleCommandContainsNul,
    MissingStopCommand,
    TrailingData(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binary(error) => error.fmt(f),
            Self::FileTooLarge { size, limit } => {
                write!(f, "demo size {size} exceeds limit {limit}")
            }
            Self::InvalidStamp(value) => write!(f, "invalid demo stamp {value:?}"),
            Self::UnsupportedDemoProtocol(value) => {
                write!(f, "unsupported demo protocol {value}")
            }
            Self::UnsupportedNetworkProtocol(value) => {
                write!(f, "unsupported network protocol {value}")
            }
            Self::UnterminatedString(field) => write!(f, "unterminated demo {field}"),
            Self::InvalidUtf8(field) => write!(f, "demo {field} is not valid UTF-8"),
            Self::InvalidHeaderValue(field) => write!(f, "invalid demo {field}"),
            Self::InvalidCommand(value) => write!(f, "invalid demo command {value}"),
            Self::CommandLimitExceeded(limit) => {
                write!(f, "demo command count exceeds {limit}")
            }
            Self::PayloadTooLarge { size, limit } => {
                write!(f, "demo payload size {size} exceeds limit {limit}")
            }
            Self::InvalidPayloadLength(value) => write!(f, "invalid demo payload length {value}"),
            Self::InvalidCommandInfoFloat => {
                write!(f, "demo command info contains a non-finite float")
            }
            Self::ConsoleCommandNotTerminated => {
                write!(f, "demo console command has no trailing NUL")
            }
            Self::ConsoleCommandContainsNul => {
                write!(f, "demo console command contains an embedded NUL")
            }
            Self::MissingStopCommand => write!(f, "demo has no stop command"),
            Self::TrailingData(size) => write!(f, "demo has {size} bytes after its stop command"),
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

fn read_packet<'a>(reader: &mut Reader<'a>, limits: Limits) -> Result<Packet<'a>> {
    let start = reader.position();
    let flags = reader.read_u32_le()?;
    let mut floats = [0.0f32; 18];
    for value in &mut floats {
        *value = reader.read_f32_le()?;
        if !value.is_finite() {
            return Err(Error::InvalidCommandInfoFloat);
        }
    }
    debug_assert_eq!(reader.position() - start, CMD_INFO_SIZE);
    let incoming_sequence = reader.read_i32_le()?;
    let outgoing_acknowledged = reader.read_i32_le()?;
    let data = read_payload(reader, limits)?;
    Ok(Packet {
        info: CommandInfo {
            flags,
            view_origin: floats[0..3].try_into().expect("three floats"),
            view_angles: floats[3..6].try_into().expect("three floats"),
            local_view_angles: floats[6..9].try_into().expect("three floats"),
            view_origin2: floats[9..12].try_into().expect("three floats"),
            view_angles2: floats[12..15].try_into().expect("three floats"),
            local_view_angles2: floats[15..18].try_into().expect("three floats"),
        },
        incoming_sequence,
        outgoing_acknowledged,
        data,
    })
}

fn read_payload<'a>(reader: &mut Reader<'a>, limits: Limits) -> Result<&'a [u8]> {
    let length = reader.read_i32_le()?;
    let size = usize::try_from(length).map_err(|_| Error::InvalidPayloadLength(length))?;
    if size > limits.max_payload_size {
        return Err(Error::PayloadTooLarge {
            size,
            limit: limits.max_payload_size,
        });
    }
    Ok(reader.take(size)?)
}

fn fixed_string<'a>(bytes: &'a [u8], field: &'static str) -> Result<&'a str> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(Error::UnterminatedString(field))?;
    std::str::from_utf8(&bytes[..end]).map_err(|_| Error::InvalidUtf8(field))
}

fn nul_string<'a>(bytes: &'a [u8], field: &'static str) -> Result<&'a str> {
    let Some((&0, contents)) = bytes.split_last() else {
        return Err(Error::ConsoleCommandNotTerminated);
    };
    if contents.contains(&0) {
        return Err(Error::ConsoleCommandContainsNul);
    }
    std::str::from_utf8(contents).map_err(|_| Error::InvalidUtf8(field))
}

fn nonnegative(field: &'static str, value: i32) -> Result<i32> {
    if value < 0 {
        Err(Error::InvalidHeaderValue(field))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> Vec<u8> {
        let mut bytes = vec![0; HEADER_SIZE];
        bytes[0..8].copy_from_slice(DEMO_STAMP);
        bytes[8..12].copy_from_slice(&DEMO_PROTOCOL.to_le_bytes());
        bytes[12..16].copy_from_slice(&NETWORK_PROTOCOL.to_le_bytes());
        for offset in [16, 276, 536, 796] {
            bytes[offset] = b'x';
            bytes[offset + 1] = 0;
        }
        bytes[1056..1060].copy_from_slice(&1.0f32.to_le_bytes());
        bytes
    }

    #[test]
    fn parses_all_command_shapes() {
        let mut bytes = header();
        bytes.extend_from_slice(&[3]);
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&[4]);
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.extend_from_slice(&5i32.to_le_bytes());
        bytes.extend_from_slice(b"echo\0");
        bytes.extend_from_slice(&[5]);
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&7i32.to_le_bytes());
        bytes.extend_from_slice(&2i32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2]);
        bytes.extend_from_slice(&[6]);
        bytes.extend_from_slice(&3i32.to_le_bytes());
        bytes.extend_from_slice(&1i32.to_le_bytes());
        bytes.push(9);
        bytes.extend_from_slice(&[8]);
        bytes.extend_from_slice(&4i32.to_le_bytes());
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&[7]);
        bytes.extend_from_slice(&5i32.to_le_bytes());
        let demo = Demo::parse(&bytes).unwrap();
        assert_eq!(demo.commands().len(), 6);
        assert!(matches!(
            demo.commands()[1].data,
            CommandData::ConsoleCommand("echo")
        ));
        assert_eq!(demo.original_bytes(), bytes);
    }

    #[test]
    fn parses_packet_command_info() {
        let mut bytes = header();
        bytes.extend_from_slice(&[2]);
        bytes.extend_from_slice(&10i32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        for value in 0..18 {
            bytes.extend_from_slice(&(value as f32).to_le_bytes());
        }
        bytes.extend_from_slice(&20i32.to_le_bytes());
        bytes.extend_from_slice(&19i32.to_le_bytes());
        bytes.extend_from_slice(&3i32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3]);
        bytes.extend_from_slice(&[7]);
        bytes.extend_from_slice(&11i32.to_le_bytes());
        let demo = Demo::parse(&bytes).unwrap();
        let CommandData::Packet(packet) = &demo.commands()[0].data else {
            panic!("expected packet");
        };
        assert_eq!(packet.info.local_view_angles2, [15.0, 16.0, 17.0]);
        assert_eq!(packet.data, &[1, 2, 3]);
    }

    #[test]
    fn round_trips_the_header_it_encodes() {
        let original = Header {
            demo_protocol: DEMO_PROTOCOL,
            network_protocol: NETWORK_PROTOCOL,
            server_name: "listen server",
            client_name: "player",
            map_name: "d1_trainstation_01",
            game_directory: "hl2",
            playback_time: 12.5,
            playback_ticks: 800,
            playback_frames: 750,
            signon_length: 0,
        };
        let encoded = original.encode().unwrap();
        assert_eq!(encoded.len(), HEADER_SIZE);

        // Appending only a stop command makes it a complete parseable demo.
        let mut bytes = encoded.clone();
        bytes.extend_from_slice(&[7]);
        bytes.extend_from_slice(&800i32.to_le_bytes());
        let demo = Demo::parse(&bytes).unwrap();
        assert_eq!(demo.header(), &original);

        // Every name field has to stay null-terminated, so a name that fills
        // it is refused rather than silently truncated.
        let overlong = "x".repeat(HEADER_NAME_SIZE);
        assert!(matches!(
            Header {
                map_name: &overlong,
                ..original
            }
            .encode(),
            Err(Error::InvalidHeaderValue("map name"))
        ));
        assert!(matches!(
            Header {
                playback_ticks: -1,
                ..original
            }
            .encode(),
            Err(Error::InvalidHeaderValue("playback ticks"))
        ));
        assert!(matches!(
            Header {
                playback_time: f32::NAN,
                ..original
            }
            .encode(),
            Err(Error::InvalidHeaderValue("playback time"))
        ));
    }

    #[test]
    fn rejects_protocol_lengths_and_missing_stop() {
        let mut bytes = header();
        bytes[12..16].copy_from_slice(&24i32.to_le_bytes());
        assert!(matches!(
            Demo::parse(&bytes),
            Err(Error::UnsupportedNetworkProtocol(24))
        ));

        let bytes = header();
        assert!(matches!(
            Demo::parse(&bytes),
            Err(Error::MissingStopCommand)
        ));

        let mut bytes = header();
        bytes.extend_from_slice(&[6]);
        bytes.extend_from_slice(&0i32.to_le_bytes());
        bytes.extend_from_slice(&(-1i32).to_le_bytes());
        assert!(matches!(
            Demo::parse(&bytes),
            Err(Error::InvalidPayloadLength(-1))
        ));
    }

    #[test]
    fn can_inspect_an_older_network_protocol_explicitly() {
        let mut bytes = header();
        bytes[12..16].copy_from_slice(&7i32.to_le_bytes());
        bytes.extend_from_slice(&[7]);
        bytes.extend_from_slice(&0i32.to_le_bytes());
        let options = ParseOptions {
            required_network_protocol: None,
            ..ParseOptions::default()
        };
        let demo = Demo::parse_with_options(&bytes, options).unwrap();
        assert_eq!(demo.header().network_protocol, 7);
    }
}
