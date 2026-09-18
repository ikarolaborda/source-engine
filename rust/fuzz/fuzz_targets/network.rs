#![no_main]

use libfuzzer_sys::fuzz_target;
use source_net::{
    finalize_packet_header, parse_packet_header, short_packet_checksum, split_packet,
    ClientMessageStream, MessageStream, ParseOptions, SplitPacketHeader, SplitPacketReassembler,
};

fuzz_target!(|data: &[u8]| {
    let _ = MessageStream::parse(data);
    let _ = ClientMessageStream::parse(data);
    let _ = short_packet_checksum(data);
    let _ = parse_packet_header(data, false, false, 0);
    let _ = parse_packet_header(data, true, true, 0xa1b2_c3d4);
    let mut packet = data.to_vec();
    let _ = finalize_packet_header(&mut packet, 0x31, true);

    let compatibility = ParseOptions {
        network_protocol: 7,
        required_network_protocol: None,
        message_type_bits: 5,
        replay_enabled: true,
        xbox_360: true,
        ..ParseOptions::default()
    };
    let _ = MessageStream::parse_with_options(data, data.len().saturating_mul(8), compatibility);
    let _ =
        ClientMessageStream::parse_with_options(data, data.len().saturating_mul(8), compatibility);

    let _ = SplitPacketHeader::decode(data);
    // Fed as a stream rather than one datagram, because the failures worth
    // finding are the ones where an earlier piece sets up state a later one
    // is placed against.
    let mut reassembler = SplitPacketReassembler::new();
    for piece in data.chunks(data.len().div_ceil(4).max(1)) {
        let _ = reassembler.accept(piece);
    }

    // Pieces the splitter itself produced must always rebuild what went in,
    // whatever the payload was. A payload it refuses is one that never had to
    // be split.
    if let Ok(pieces) = split_packet(data, 1, 1260) {
        let mut round_trip = SplitPacketReassembler::new();
        let mut rebuilt = None;
        for piece in &pieces {
            rebuilt = round_trip.accept(piece).expect("own pieces are placeable");
        }
        assert_eq!(rebuilt.as_deref(), Some(data));
    }
});
