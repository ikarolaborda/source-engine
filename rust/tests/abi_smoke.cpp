#include "appframework/rust_engine_bridge.h"

#include <algorithm>
#include <cstdio>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>

static int g_LogCalls = 0;
static int g_SessionCalls = 0;
static int g_FrameCalls = 0;

static void LogCallback(void *, int32_t level, SourceAbiSlice message)
{
	if (level == 4 && message.length == 5 && std::memcmp(message.data, "hello", 5) == 0)
		++g_LogCalls;
}

static int32_t SessionCallback(void *)
{
	++g_SessionCalls;
	return g_SessionCalls < 3 ? SOURCE_HOST_SESSION_RESTART : SOURCE_HOST_SESSION_STOP;
}

static int32_t FrameCallback(void *)
{
	++g_FrameCalls;
	return g_FrameCalls < 4 ? SOURCE_HOST_FRAME_CONTINUE : SOURCE_HOST_FRAME_STOP;
}

static int32_t NestedFrameSessionCallback(void *)
{
	SourceAbiFrameLoopInfo info = {};
	return source_rust_bridge_host_run_frames(FrameCallback, 0, 8, &info) == SOURCE_ABI_OK &&
		info.iteration_count == 4 && info.exit_reason == SOURCE_HOST_FRAME_STOP
		? SOURCE_HOST_SESSION_STOP
		: -1;
}

int main(int argc, char **argv)
{
	if (argc > 2)
		return 15;
	if (source_abi_version() != SOURCE_ABI_VERSION)
		return 1;

	for (int i = 0; i < 10000; ++i)
	{
		CRustEngineBridge bridge;
		if (bridge.Init(LogCallback, 0) != SOURCE_ABI_OK)
			return 2;
		if (source_rust_bridge_activate(bridge.Handle()) != SOURCE_ABI_OK)
			return 43;
		const char input[] = "adapter";
		char output[sizeof(input)] = {};
		uint64_t written = 0;
		if (bridge.Echo(input, sizeof(input), output, sizeof(output), &written) != SOURCE_ABI_OK)
			return 3;
		if (written != sizeof(input) || std::memcmp(input, output, sizeof(input)) != 0)
			return 4;
		static const char testRoot[] = "/tmp/game";
		static const char testVirtualPath[] = "hl2/cfg/valve.rc";
		static const char testResolvedPath[] = "/tmp/game/hl2/cfg/valve.rc";
		char resolved[4096];
		if (bridge.ResolveContentPath(testRoot, sizeof(testRoot) - 1,
			testVirtualPath, sizeof(testVirtualPath) - 1,
			resolved, sizeof(resolved), &written) != SOURCE_ABI_OK)
			return 19;
		if (written != sizeof(testResolvedPath) - 1 ||
			std::memcmp(resolved, testResolvedPath, sizeof(testResolvedPath) - 1) != 0)
			return 20;
		if (bridge.ExecutableBase(resolved, sizeof(resolved), &written) != SOURCE_ABI_OK ||
			written == 0 || written >= sizeof(resolved))
			return 29;
		if (i == 0)
		{
			if (source_rust_bridge_write_paths_clear() != SOURCE_ABI_OK ||
				source_rust_bridge_write_path_add("/tmp", 4, "GAME_WRITE", 10,
					false) != SOURCE_ABI_OK)
				return 100;
			char writePath[4096] = {};
			uint64_t writePathBytes = 0;
			static const char writeVirtualPath[] = "cfg/rust-write.cfg";
			if (source_rust_bridge_resolve_write_path(writeVirtualPath,
				sizeof(writeVirtualPath) - 1, "GAME", 4, writePath,
				sizeof(writePath), &writePathBytes) != SOURCE_ABI_OK ||
				writePathBytes < sizeof(writeVirtualPath) || writePath[0] != '/' ||
				std::string(writePath, writePathBytes).find(writeVirtualPath) == std::string::npos)
				return 101;
			char tinyWritePath[1] = {};
			if (source_rust_bridge_resolve_write_path(writeVirtualPath,
				sizeof(writeVirtualPath) - 1, "GAME", 4, tinyWritePath,
				sizeof(tinyWritePath), &writePathBytes) != SOURCE_ABI_BUFFER_TOO_SMALL ||
				writePathBytes == 0)
				return 102;
			if (source_rust_bridge_resolve_write_path("../escape", 9, "GAME", 4,
				writePath, sizeof(writePath), &writePathBytes) != SOURCE_ABI_FORMAT_ERROR)
				return 103;
			if (source_rust_bridge_write_paths_clear() != SOURCE_ABI_OK ||
				source_rust_bridge_resolve_write_path(writeVirtualPath,
					sizeof(writeVirtualPath) - 1, "GAME", 4, writePath,
					sizeof(writePath), &writePathBytes) != SOURCE_ABI_NOT_FOUND)
				return 104;
			static const char abiWriteDirectory[] = "source-rust-abi-directory/";
			static const char abiWriteName[] = "source-rust-abi-directory/state.tmp";
			static const char abiRenamedName[] = "source-rust-abi-directory/renamed.tmp";
			std::remove("/tmp/source-rust-abi-directory/state.tmp");
			std::remove("/tmp/source-rust-abi-directory/renamed.tmp");
			std::remove("/tmp/source-rust-abi-directory");
			if (source_rust_bridge_write_path_add("/tmp", 4, "GAME_WRITE", 10,
				false) != SOURCE_ABI_OK ||
				source_rust_bridge_create_write_directory(abiWriteDirectory,
					sizeof(abiWriteDirectory) - 1, "GAME", 4) != SOURCE_ABI_OK)
				return 105;
			uint64_t rustFile = 0;
			uint64_t rustFileSize = 99;
			if (source_rust_bridge_file_open_write(abiWriteName,
				sizeof(abiWriteName) - 1, "GAME", 4, "w+b", 3,
				&rustFile, &rustFileSize) != SOURCE_ABI_OK || rustFile == 0 || rustFileSize != 0)
				return 106;
			uint32_t rustFileOpen = 0;
			if (source_rust_bridge_file_is_open(rustFile, &rustFileOpen) != SOURCE_ABI_OK ||
				rustFileOpen != 1)
				return 107;
			static const char abiWritePayload[] = "rust-file";
			uint64_t rustFileBytes = 0;
			if (source_rust_bridge_file_write(rustFile, abiWritePayload,
				sizeof(abiWritePayload) - 1, &rustFileBytes) != SOURCE_ABI_OK ||
				rustFileBytes != sizeof(abiWritePayload) - 1 ||
				source_rust_bridge_file_flush(rustFile) != SOURCE_ABI_OK ||
				source_rust_bridge_file_tell(rustFile, &rustFileBytes) != SOURCE_ABI_OK ||
				rustFileBytes != sizeof(abiWritePayload) - 1 ||
				source_rust_bridge_open_file_size(rustFile, &rustFileSize) != SOURCE_ABI_OK ||
				rustFileSize != sizeof(abiWritePayload) - 1)
				return 108;
			if (source_rust_bridge_file_seek(rustFile, 0, 0, &rustFileBytes) != SOURCE_ABI_OK ||
				rustFileBytes != 0)
				return 109;
			char abiReadback[sizeof(abiWritePayload)] = {};
			if (source_rust_bridge_file_read(rustFile, abiReadback,
				sizeof(abiWritePayload) - 1, &rustFileBytes) != SOURCE_ABI_OK ||
				rustFileBytes != sizeof(abiWritePayload) - 1 ||
				std::memcmp(abiReadback, abiWritePayload, sizeof(abiWritePayload) - 1) != 0)
				return 110;
			if (source_rust_bridge_file_close(rustFile) != SOURCE_ABI_OK ||
				source_rust_bridge_file_is_open(rustFile, &rustFileOpen) != SOURCE_ABI_OK ||
				rustFileOpen != 0 || source_rust_bridge_file_close(rustFile) != SOURCE_ABI_NOT_FOUND)
				return 111;
			uint32_t rustFileWritable = 0;
			if (source_rust_bridge_is_write_file_writable(abiWriteName,
				sizeof(abiWriteName) - 1, "GAME_WRITE", 10,
				&rustFileWritable) != SOURCE_ABI_OK || rustFileWritable != 1)
				return 112;
			if (source_rust_bridge_set_write_file_writable(abiWriteName,
				sizeof(abiWriteName) - 1, "GAME_WRITE", 10, false) != SOURCE_ABI_OK ||
				source_rust_bridge_is_write_file_writable(abiWriteName,
					sizeof(abiWriteName) - 1, "GAME_WRITE", 10,
					&rustFileWritable) != SOURCE_ABI_OK || rustFileWritable != 0 ||
				source_rust_bridge_set_write_file_writable(abiWriteName,
					sizeof(abiWriteName) - 1, "GAME_WRITE", 10, true) != SOURCE_ABI_OK)
				return 113;
			if (source_rust_bridge_rename_write_file(abiWriteName,
				sizeof(abiWriteName) - 1, "GAME_WRITE", 10, abiRenamedName,
				sizeof(abiRenamedName) - 1, "GAME", 4) != SOURCE_ABI_OK)
				return 114;
			if (source_rust_bridge_remove_write_file(abiRenamedName,
				sizeof(abiRenamedName) - 1, "GAME", 4) != SOURCE_ABI_OK)
				return 115;
			std::remove("/tmp/source-rust-abi-directory");

			uint32_t hostPhase = UINT32_MAX;
			if (bridge.HostPhase(&hostPhase) != SOURCE_ABI_OK || hostPhase != SOURCE_HOST_CREATED)
				return 30;
			if (bridge.HostTransition(SOURCE_HOST_CONTENT_READY) != SOURCE_ABI_FORMAT_ERROR)
				return 31;
			if (bridge.HostTransition(SOURCE_HOST_LAUNCHER_READY) != SOURCE_ABI_OK)
				return 32;
			if (bridge.HostConfigureScheduler(15000000, 4) != SOURCE_ABI_OK)
				return 33;
			SourceAbiFramePlan framePlan = {};
			if (bridge.HostAdvance(1000, &framePlan) != SOURCE_ABI_OK || framePlan.tick_count != 0)
				return 34;
			if (bridge.HostAdvance(22501000, &framePlan) != SOURCE_ABI_OK ||
				framePlan.tick_count != 1 || framePlan.interpolation_numerator_ns != 7500000 ||
				framePlan.interpolation_denominator_ns != 15000000 || framePlan.dropped_ns != 0)
				return 35;
			SourceAbiFramePace framePace = {};
			if (bridge.HostPace(1000, 15000000, &framePace) != SOURCE_ABI_OK ||
				framePace.ready != 1 || framePace.elapsed_ns != 15000000 ||
				framePace.wait_ns != 0)
				return 54;
			if (bridge.HostPace(10001000, 15000000, &framePace) != SOURCE_ABI_OK ||
				framePace.ready != 0 || framePace.elapsed_ns != 10000000 ||
				framePace.wait_ns != 5000000)
				return 55;

			static const char cvarName[] = "rust_host_name";
			if (bridge.CvarRegister(cvarName, sizeof(cvarName) - 1, "lambda", 6, 0) != SOURCE_ABI_OK)
				return 45;
			SourceAbiCvarInfo cvarInfo = {};
			uint64_t cvarBytes = 0;
			if (bridge.CvarGet(cvarName, sizeof(cvarName) - 1, 0, 0,
				&cvarBytes, &cvarInfo) != SOURCE_ABI_BUFFER_TOO_SMALL || cvarBytes != 6 ||
				cvarInfo.generation != 0)
				return 46;
			if (bridge.CvarSet(cvarName, sizeof(cvarName) - 1, "rust", 4) != SOURCE_ABI_OK)
				return 47;
			char cvarValue[16] = {};
			if (bridge.CvarGet(cvarName, sizeof(cvarName) - 1, cvarValue, sizeof(cvarValue),
				&cvarBytes, &cvarInfo) != SOURCE_ABI_OK || cvarBytes != 4 ||
				std::memcmp(cvarValue, "rust", 4) != 0 || cvarInfo.generation != 1)
				return 48;
			uint32_t commandCount = 0;
			static const char commands[] = "echo \"hello world\";quit";
			if (source_rust_bridge_command_enqueue(commands, sizeof(commands) - 1,
				&commandCount) != SOURCE_ABI_OK ||
				commandCount != 2)
				return 49;
			char command[64] = {};
			uint64_t commandBytes = 0;
			if (source_rust_bridge_command_pop(command, sizeof(command),
				&commandBytes) != SOURCE_ABI_OK ||
				commandBytes != 18 || std::memcmp(command, "echo \"hello world\"", 18) != 0)
				return 50;
			if (source_rust_bridge_command_pop(command, sizeof(command),
				&commandBytes) != SOURCE_ABI_OK ||
				commandBytes != 4 || std::memcmp(command, "quit", 4) != 0)
				return 51;
			if (source_rust_bridge_command_pop(command, sizeof(command),
				&commandBytes) != SOURCE_ABI_NOT_FOUND)
				return 52;

			uint64_t channelId = 0;
			if (source_rust_bridge_net_channel_create(1, 0, 0,
				&channelId) != SOURCE_ABI_OK || channelId == 0)
				return 65;
			SourceAbiNetSequenceAdvance sequenceAdvance = {};
			if (source_rust_bridge_net_channel_advance_outgoing(channelId,
				&sequenceAdvance) != SOURCE_ABI_OK || sequenceAdvance.previous != 1 ||
				sequenceAdvance.current != 2)
				return 66;
			SourceAbiNetPacketDecision packetDecision = {};
			if (source_rust_bridge_net_channel_preview_incoming(channelId, 5, 3, 1, 100,
				&packetDecision) != SOURCE_ABI_OK || packetDecision.accepted != 1 ||
				packetDecision.dropped != 3 || packetDecision.reason != SOURCE_NET_PACKET_ACCEPTED)
				return 67;
			if (source_rust_bridge_net_channel_commit_incoming(channelId, 5, 3, 1, 100,
				&packetDecision) != SOURCE_ABI_OK || packetDecision.incoming_sequence != 5 ||
				packetDecision.outgoing_ack != 3)
				return 68;
			if (source_rust_bridge_net_channel_preview_incoming(channelId, 5, 4, 0, 100,
				&packetDecision) != SOURCE_ABI_OK || packetDecision.accepted != 0 ||
				packetDecision.reason != SOURCE_NET_PACKET_DUPLICATE)
				return 69;
			uint16_t packetChecksum = 0;
			static const char checksumVector[] = "123456789";
			if (source_rust_bridge_net_packet_checksum(checksumVector,
				sizeof(checksumVector) - 1, &packetChecksum) != SOURCE_ABI_OK ||
				packetChecksum != 0xf2d2)
				return 97;
			SourceAbiNetEncodedPacketHeader encodedHeader = {};
			if (source_rust_bridge_net_packet_encode_header(17, 11, 0xa5, true, 3,
				true, 0xa1b2c3d4, true, &encodedHeader) != SOURCE_ABI_OK ||
				encodedHeader.length != 17 || encodedHeader.flags_offset != 8 ||
				encodedHeader.checksum_offset != 9 || encodedHeader.checksum_start != 11)
				return 97;
			uint8_t packet[24] = {};
			std::memcpy(packet, encodedHeader.bytes, encodedHeader.length);
			std::memcpy(packet + encodedHeader.length, "payload", 7);
			if (source_rust_bridge_net_packet_finalize_header(packet, sizeof(packet),
				encodedHeader.base_flags | 1, true, &packetChecksum) != SOURCE_ABI_OK ||
				packetChecksum == 0)
				return 97;
			SourceAbiNetPacketHeader packetHeader = {};
			if (source_rust_bridge_net_packet_header(packet, sizeof(packet), true, false,
				0xa1b2c3d4, &packetHeader) != SOURCE_ABI_OK || packetHeader.accepted != 1 ||
				packetHeader.reason != SOURCE_NET_HEADER_ACCEPTED ||
				packetHeader.sequence != 17 || packetHeader.outgoing_ack != 11 ||
				packetHeader.reliable_state != 0xa5 || packetHeader.choked != 3 ||
				packetHeader.challenge != 0xa1b2c3d4 || packetHeader.header_bytes != 17)
				return 98;
			packet[sizeof(packet) - 1] ^= 1;
			if (source_rust_bridge_net_packet_header(packet, sizeof(packet), true, false,
				0xa1b2c3d4, &packetHeader) != SOURCE_ABI_OK || packetHeader.accepted != 0 ||
				packetHeader.reason != SOURCE_NET_HEADER_CHECKSUM_MISMATCH)
				return 99;
			if (source_rust_bridge_net_channel_remove(channelId) != SOURCE_ABI_OK ||
				source_rust_bridge_net_channel_remove(channelId) != SOURCE_ABI_NOT_FOUND)
				return 70;

			// A datagram too large to route is cut into pieces that each
			// carry their position, and only rebuilds once all of them land.
			uint64_t splitPeer = 0;
			if (source_rust_bridge_split_packet_create(&splitPeer) != SOURCE_ABI_OK ||
				splitPeer == 0)
				return 100;
			{
				const uint32_t splitSize = 1248;
				const uint32_t pieceCount = 3;
				std::vector<uint8_t> whole(splitSize * 2 + 100);
				for (size_t index = 0; index < whole.size(); ++index)
					whole[index] = static_cast<uint8_t>(index % 251);

				std::vector<std::vector<uint8_t> > pieces;
				for (uint32_t piece = 0; piece < pieceCount; ++piece)
				{
					const size_t offset = piece * splitSize;
					const size_t bytes = std::min<size_t>(splitSize, whole.size() - offset);
					std::vector<uint8_t> datagram(12 + bytes);
					if (source_rust_bridge_split_packet_header_encode(4242, piece,
						pieceCount, splitSize, datagram.data()) != SOURCE_ABI_OK)
						return 101;
					std::memcpy(datagram.data() + 12, whole.data() + offset, bytes);
					pieces.push_back(datagram);
				}

				SourceAbiSplitPacketHeader splitHeader = {};
				if (source_rust_bridge_split_packet_header_decode(pieces[1].data(),
					pieces[1].size(), &splitHeader) != SOURCE_ABI_OK ||
					splitHeader.sequence != 4242 || splitHeader.packet_number != 1 ||
					splitHeader.packet_count != pieceCount ||
					splitHeader.split_size != splitSize)
					return 102;

				std::vector<uint8_t> rebuilt(whole.size());
				uint64_t rebuiltLength = 0;
				bool rebuiltComplete = false;
				for (size_t piece = 0; piece + 1 < pieces.size(); ++piece)
				{
					if (source_rust_bridge_split_packet_accept(splitPeer,
						pieces[piece].data(), pieces[piece].size(), rebuilt.data(),
						rebuilt.size(), &rebuiltLength, &rebuiltComplete) != SOURCE_ABI_OK ||
						rebuiltComplete)
						return 103;
				}
				// The last piece completes it, but into a buffer one byte too
				// small, so the message has to be held rather than lost.
				if (source_rust_bridge_split_packet_accept(splitPeer, pieces.back().data(),
					pieces.back().size(), rebuilt.data(), rebuilt.size() - 1,
					&rebuiltLength, &rebuiltComplete) != SOURCE_ABI_BUFFER_TOO_SMALL ||
					rebuiltComplete || rebuiltLength != whole.size())
					return 104;
				rebuiltLength = 0;
				if (source_rust_bridge_split_packet_collect(splitPeer, rebuilt.data(),
					rebuilt.size(), &rebuiltLength) != SOURCE_ABI_OK ||
					rebuiltLength != whole.size() || rebuilt != whole)
					return 105;

				// A piece whose leading word is not the split flag is not one.
				std::vector<uint8_t> notSplit = pieces[0];
				notSplit[0] = 0;
				if (source_rust_bridge_split_packet_accept(splitPeer, notSplit.data(),
					notSplit.size(), rebuilt.data(), rebuilt.size(), &rebuiltLength,
					&rebuiltComplete) != SOURCE_ABI_FORMAT_ERROR || rebuiltComplete)
					return 106;
			}
			if (source_rust_bridge_split_packet_reset(splitPeer) != SOURCE_ABI_OK ||
				source_rust_bridge_split_packet_remove(splitPeer) != SOURCE_ABI_OK ||
				source_rust_bridge_split_packet_remove(splitPeer) !=
					SOURCE_ABI_INVALID_ARGUMENT)
				return 107;

			uint64_t stringTableId = 0;
			static const char stringTableName[] = "modelprecache";
			if (source_rust_bridge_string_table_create(stringTableName,
				sizeof(stringTableName) - 1, 4, 1, &stringTableId) != SOURCE_ABI_OK ||
				stringTableId == 0)
				return 71;
			if (source_rust_bridge_string_table_enable_history(stringTableId) != SOURCE_ABI_OK)
				return 72;
			SourceAbiStringTableUpsert stringUpsert = {};
			static const char stringValue[] = "models/gman_high.mdl";
			if (source_rust_bridge_string_table_upsert(stringTableId, stringValue,
				sizeof(stringValue) - 1, true, "one", 3, &stringUpsert) != SOURCE_ABI_OK ||
				stringUpsert.index != 0 || stringUpsert.created != 1)
				return 73;
			uint32_t stringIndex = UINT32_MAX;
			if (source_rust_bridge_string_table_find(stringTableId,
				"MODELS/GMAN_HIGH.MDL", sizeof(stringValue) - 1,
				&stringIndex) != SOURCE_ABI_OK || stringIndex != 0)
				return 74;
			char canonicalString[32] = {};
			uint64_t canonicalBytes = 0;
			if (source_rust_bridge_string_table_read_string(stringTableId, 0,
				canonicalString, sizeof(canonicalString), &canonicalBytes) != SOURCE_ABI_OK ||
				canonicalBytes != sizeof(stringValue) - 1 ||
				std::memcmp(canonicalString, stringValue, sizeof(stringValue) - 1) != 0)
				return 75;
			if (source_rust_bridge_string_table_set_tick(stringTableId, 2) != SOURCE_ABI_OK)
				return 76;
			SourceAbiStringTableChange stringChange = {};
			if (source_rust_bridge_string_table_set_user_data(stringTableId, 0,
				"two", 3, &stringChange) != SOURCE_ABI_OK || stringChange.changed != 0)
				return 77;
			int32_t lastChangedTick = 0;
			if (source_rust_bridge_string_table_restore_tick(stringTableId, 1,
				&lastChangedTick) != SOURCE_ABI_OK || lastChangedTick != 1)
				return 78;
			char canonicalUserData[8] = {};
			if (source_rust_bridge_string_table_read_user_data(stringTableId, 0,
				canonicalUserData, sizeof(canonicalUserData), &canonicalBytes) != SOURCE_ABI_OK ||
				canonicalBytes != 3 || std::memcmp(canonicalUserData, "one", 3) != 0)
				return 79;
			uint32_t stringEntryCount = 0;
			if (source_rust_bridge_string_table_entry_count(stringTableId,
				&stringEntryCount) != SOURCE_ABI_OK || stringEntryCount != 1 ||
				source_rust_bridge_string_table_remove(stringTableId) != SOURCE_ABI_OK)
				return 80;

			if (source_rust_bridge_data_table_clear() != SOURCE_ABI_OK)
				return 81;
			SourceAbiDataTableRegistration baseTable = {};
			static const char baseTableName[] = "DT_BaseEntity";
			if (source_rust_bridge_data_table_register(baseTableName,
				sizeof(baseTableName) - 1, 1, &baseTable) != SOURCE_ABI_OK ||
				baseTable.table_id != 0 || baseTable.created != 1)
				return 82;
			static const char basePropertyName[] = "m_iTeamNum";
			SourceAbiDataTableProperty baseProperty = {};
			baseProperty.name.data = reinterpret_cast<const uint8_t *>(basePropertyName);
			baseProperty.name.length = sizeof(basePropertyName) - 1;
			baseProperty.property_type = 0;
			baseProperty.flags = 1;
			baseProperty.bit_count = 8;
			baseProperty.elements = 1;
			baseProperty.high_value = 255.0f;
			if (source_rust_bridge_data_table_register_property(baseTable.table_id,
				&baseProperty) != SOURCE_ABI_OK)
				return 83;
			SourceAbiDataTableRegistration derivedTable = {};
			static const char derivedTableName[] = "DT_TestEntity";
			if (source_rust_bridge_data_table_register(derivedTableName,
				sizeof(derivedTableName) - 1, 1, &derivedTable) != SOURCE_ABI_OK ||
				derivedTable.table_id != 1 || derivedTable.created != 1)
				return 84;
			static const char derivedPropertyName[] = "baseclass";
			SourceAbiDataTableProperty derivedProperty = {};
			derivedProperty.name.data = reinterpret_cast<const uint8_t *>(derivedPropertyName);
			derivedProperty.name.length = sizeof(derivedPropertyName) - 1;
			derivedProperty.reference_name.data =
				reinterpret_cast<const uint8_t *>(baseTableName);
			derivedProperty.reference_name.length = sizeof(baseTableName) - 1;
			derivedProperty.property_type = 6;
			derivedProperty.flags = 1u << 12;
			derivedProperty.elements = 1;
			if (source_rust_bridge_data_table_register_property(derivedTable.table_id,
				&derivedProperty) != SOURCE_ABI_OK)
				return 85;
			static const char serverClassName[] = "CTestEntity";
			uint32_t serverClassId = UINT32_MAX;
			if (source_rust_bridge_server_class_register(serverClassName,
				sizeof(serverClassName) - 1, derivedTable.table_id,
				&serverClassId) != SOURCE_ABI_OK || serverClassId != 0)
				return 86;
			SourceAbiDataTableSummary dataTableSummary = {};
			if (source_rust_bridge_data_table_finalize(0xf7bb8e46u,
				&dataTableSummary) != SOURCE_ABI_OK || dataTableSummary.table_count != 2 ||
				dataTableSummary.property_count != 2 || dataTableSummary.class_count != 1 ||
				dataTableSummary.compatibility_crc != 0xf7bb8e46u)
				return 87;
			if (source_rust_bridge_server_class_find("ctestentity",
				sizeof(serverClassName) - 1, &serverClassId) != SOURCE_ABI_OK ||
				serverClassId != 0)
				return 88;

			uint32_t snapshotQueued = 0;
			if (source_rust_bridge_snapshot_queue_delete(3, &snapshotQueued) != SOURCE_ABI_OK ||
				snapshotQueued != 1)
				return 89;
			SourceAbiSnapshotEntity snapshotEntities[2] = {};
			snapshotEntities[0].entity_index = 0;
			snapshotEntities[0].serial_number = 11;
			snapshotEntities[0].class_id = 0;
			snapshotEntities[1].entity_index = 7;
			snapshotEntities[1].serial_number = 23;
			snapshotEntities[1].class_id = 0;
			uint64_t snapshotId = 0;
			SourceAbiSnapshotSummary snapshotSummary = {};
			if (source_rust_bridge_snapshot_create(42, 8, snapshotEntities, 2,
				&snapshotId, &snapshotSummary) != SOURCE_ABI_OK || snapshotId == 0 ||
				snapshotSummary.tick != 42 || snapshotSummary.max_entities != 8 ||
				snapshotSummary.valid_entity_count != 2 ||
				snapshotSummary.explicit_delete_count != 1)
				return 90;
			SourceAbiSnapshotEntity snapshotEntity = {};
			if (source_rust_bridge_snapshot_entity_at(snapshotId, 1,
				&snapshotEntity) != SOURCE_ABI_OK || snapshotEntity.entity_index != 7 ||
				snapshotEntity.serial_number != 23 || snapshotEntity.class_id != 0)
				return 91;
			uint32_t snapshotDelete = UINT32_MAX;
			if (source_rust_bridge_snapshot_delete_at(snapshotId, 0,
				&snapshotDelete) != SOURCE_ABI_OK || snapshotDelete != 3)
				return 92;
			SourceAbiSnapshotSummary queriedSnapshotSummary = {};
			if (source_rust_bridge_snapshot_summary(snapshotId,
				&queriedSnapshotSummary) != SOURCE_ABI_OK ||
				queriedSnapshotSummary.valid_entity_count != 2)
				return 93;

			// Entity 0 keeps its identity, entity 7 is reused under a new
			// serial, and entity 5 appears for the first time.
			SourceAbiSnapshotEntity nextEntities[3] = {};
			nextEntities[0].entity_index = 0;
			nextEntities[0].serial_number = 11;
			nextEntities[1].entity_index = 5;
			nextEntities[1].serial_number = 31;
			nextEntities[2].entity_index = 7;
			nextEntities[2].serial_number = 24;
			uint64_t nextSnapshotId = 0;
			SourceAbiSnapshotSummary nextSnapshotSummary = {};
			if (source_rust_bridge_snapshot_create(43, 8, nextEntities, 3,
				&nextSnapshotId, &nextSnapshotSummary) != SOURCE_ABI_OK ||
				nextSnapshotId == 0)
				return 97;
			SourceAbiSnapshotDelta snapshotDeltas[4] = {};
			uint64_t snapshotDeltaCount = 0;
			if (source_rust_bridge_snapshot_delta(snapshotId, NULL, 0, nextSnapshotId,
				NULL, 0, snapshotDeltas, 4, &snapshotDeltaCount) != SOURCE_ABI_OK ||
				snapshotDeltaCount != 3 ||
				snapshotDeltas[0].entity_index != 0 ||
				snapshotDeltas[0].kind != SOURCE_SNAPSHOT_DELTA_CANDIDATE ||
				snapshotDeltas[1].entity_index != 5 ||
				snapshotDeltas[1].kind != SOURCE_SNAPSHOT_DELTA_ENTER_PVS ||
				snapshotDeltas[1].recreated != 0 ||
				snapshotDeltas[2].entity_index != 7 ||
				snapshotDeltas[2].kind != SOURCE_SNAPSHOT_DELTA_ENTER_PVS ||
				snapshotDeltas[2].recreated != 1)
				return 98;
			// A short buffer still reports the size the caller needs.
			snapshotDeltaCount = 0;
			if (source_rust_bridge_snapshot_delta(snapshotId, NULL, 0, nextSnapshotId,
				NULL, 0, snapshotDeltas, 2,
				&snapshotDeltaCount) != SOURCE_ABI_BUFFER_TOO_SMALL ||
				snapshotDeltaCount != 3)
				return 99;
			// Narrowing to a visibility set drops entity 0 and keeps entity 7.
			const uint32_t visibleFrom[1] = { 7 };
			const uint32_t visibleTo[2] = { 5, 7 };
			snapshotDeltaCount = 0;
			if (source_rust_bridge_snapshot_delta(snapshotId, visibleFrom, 1,
				nextSnapshotId, visibleTo, 2, snapshotDeltas, 4,
				&snapshotDeltaCount) != SOURCE_ABI_OK || snapshotDeltaCount != 2 ||
				snapshotDeltas[0].entity_index != 5 ||
				snapshotDeltas[1].entity_index != 7 ||
				snapshotDeltas[1].recreated != 1)
				return 100;
			// A full update creates every visible entity.
			snapshotDeltaCount = 0;
			if (source_rust_bridge_snapshot_delta(0, NULL, 0, nextSnapshotId, NULL, 0,
				snapshotDeltas, 4, &snapshotDeltaCount) != SOURCE_ABI_OK ||
				snapshotDeltaCount != 3 ||
				snapshotDeltas[0].kind != SOURCE_SNAPSHOT_DELTA_ENTER_PVS ||
				snapshotDeltas[2].kind != SOURCE_SNAPSHOT_DELTA_ENTER_PVS)
				return 101;
			// A descending visibility set is rejected rather than misread.
			const uint32_t descending[2] = { 7, 5 };
			if (source_rust_bridge_snapshot_delta(0, NULL, 0, nextSnapshotId, descending,
				2, snapshotDeltas, 4,
				&snapshotDeltaCount) != SOURCE_ABI_INVALID_ARGUMENT)
				return 102;
			if (source_rust_bridge_snapshot_remove(nextSnapshotId) != SOURCE_ABI_OK)
				return 103;

			// Entity 6 after a header base of 1 is a gap of 4, which the
			// narrowest selector encodes in four payload bits, so the whole
			// header is eight bits: selector 0, gap 4, no removal, a creation.
			uint8_t headerBytes[SOURCE_MAX_DELTA_HEADER_BYTES] = {};
			uint32_t headerBits = 0;
			if (source_rust_bridge_delta_header_encode(6, 1, false, false, true,
				headerBytes, sizeof(headerBytes), &headerBits) != SOURCE_ABI_OK ||
				headerBits != 8 || headerBytes[0] != 0x90)
				return 104;
			// A removal that frees the slot sets both trailing bits.
			headerBits = 0;
			if (source_rust_bridge_delta_header_encode(0, -1, true, true, false,
				headerBytes, sizeof(headerBytes), &headerBits) != SOURCE_ABI_OK ||
				headerBits != 8 || headerBytes[0] != 0xc0)
				return 105;
			// An entity index that does not follow its header base is rejected.
			if (source_rust_bridge_delta_header_encode(4, 4, false, false, false,
				headerBytes, sizeof(headerBytes),
				&headerBits) != SOURCE_ABI_INVALID_ARGUMENT)
				return 106;
			// A short buffer still reports the width the caller needs.
			headerBits = 0;
			if (source_rust_bridge_delta_header_encode(6, 1, false, false, true,
				headerBytes, 0, &headerBits) != SOURCE_ABI_BUFFER_TOO_SMALL ||
				headerBits != 8)
				return 107;

			// The demo header is a fixed 1072 bytes starting with the stamp the
			// parser requires, and its fields sit at fixed offsets.
			uint8_t demoHeader[1072] = {};
			uint64_t demoHeaderSize = 0;
			if (source_rust_bridge_demo_header_encode(3, 25, "listen", "player",
				"d1_trainstation_01", "hl2", 4.0f, 256, 240, 0, demoHeader,
				sizeof(demoHeader), &demoHeaderSize) != SOURCE_ABI_OK ||
				demoHeaderSize != sizeof(demoHeader) ||
				std::memcmp(demoHeader, "HL2DEMO\0", 8) != 0 ||
				std::string(reinterpret_cast<const char *>(demoHeader) + 16 + 260 * 2) !=
					"d1_trainstation_01")
				return 108;
			// A name that would fill its fixed field leaves no terminator, so
			// the encoder refuses it rather than truncating.
			const std::string overlongMap(260, 'x');
			if (source_rust_bridge_demo_header_encode(3, 25, "listen", "player",
				overlongMap.c_str(), "hl2", 4.0f, 256, 240, 0, demoHeader,
				sizeof(demoHeader),
				&demoHeaderSize) != SOURCE_ABI_INVALID_ARGUMENT)
				return 109;
			// A short buffer still reports the fixed size.
			demoHeaderSize = 0;
			if (source_rust_bridge_demo_header_encode(3, 25, "listen", "player",
				"d1_trainstation_01", "hl2", 4.0f, 256, 240, 0, demoHeader, 16,
				&demoHeaderSize) != SOURCE_ABI_BUFFER_TOO_SMALL ||
				demoHeaderSize != sizeof(demoHeader))
				return 110;

			if (source_rust_bridge_snapshot_remove(snapshotId) != SOURCE_ABI_OK)
				return 94;
			if (source_rust_bridge_snapshot_remove(snapshotId) != SOURCE_ABI_NOT_FOUND)
				return 95;
			if (source_rust_bridge_snapshot_clear() != SOURCE_ABI_OK ||
				source_rust_bridge_data_table_clear() != SOURCE_ABI_OK)
				return 96;

			uint32_t inputMask = 0;
			if (bridge.InputModifier(SOURCE_INPUT_MOD_LEFT_SHIFT, true, &inputMask) != SOURCE_ABI_OK ||
				inputMask != (1u << 1))
				return 24;
			if (bridge.InputMouseButton(1u << 2, true, &inputMask) != SOURCE_ABI_OK ||
				inputMask != (1u << 2))
				return 25;
			if (bridge.InputMouseMotion(8, -3) != SOURCE_ABI_OK)
				return 26;
			int32_t deltaX = 0;
			int32_t deltaY = 0;
			if (bridge.InputTakeMouseDelta(&deltaX, &deltaY) != SOURCE_ABI_OK ||
				deltaX != 8 || deltaY != -3)
				return 27;
			if (bridge.InputReset() != SOURCE_ABI_OK)
				return 28;
			if (bridge.EmitLog(4, "hello", 5) != SOURCE_ABI_OK)
				return 5;
			if (source_context_force_panic_for_test(bridge.HandleForTesting()) != SOURCE_ABI_PANIC)
				return 6;
			uint64_t entryCount = 99;
			uint32_t version = 99;
			static const char missingVpk[] = "source_abi_missing_test.vpk";
			if (bridge.ProbeVpk(missingVpk, sizeof(missingVpk) - 1,
				&entryCount, &version) != SOURCE_ABI_IO_ERROR)
				return 9;
			if (entryCount != 0 || version != 0)
				return 10;
			uint64_t required = 99;
			if (bridge.ReadVpkFile(missingVpk, sizeof(missingVpk) - 1,
				"missing", 7, 0, 0, &required) != SOURCE_ABI_IO_ERROR)
				return 11;
			if (required != 0)
				return 12;
			uint64_t sceneCount = 99;
			uint64_t stringCount = 99;
			if (bridge.ProbeVpkScene(missingVpk, sizeof(missingVpk) - 1,
				"scenes/scenes.image", 19, &sceneCount, &stringCount) != SOURCE_ABI_IO_ERROR)
				return 13;
			if (sceneCount != 0 || stringCount != 0)
				return 14;

			if (argc == 2)
			{
				const std::string root(argv[1]);
				const std::string hl2Root = root + "/hl2";
				if (source_rust_bridge_read_paths_clear() != SOURCE_ABI_OK ||
					source_rust_bridge_read_path_add_directory_flags(hl2Root.c_str(),
						hl2Root.size(), "GAME", 4, true, false, true) != SOURCE_ABI_OK)
					return 21;
				static const char worldPath[] = "maps/d1_trainstation_01.bsp";
				SourceAbiWorldInfo worldInfo = {};
				if (source_rust_bridge_world_load(worldPath, sizeof(worldPath) - 1,
					&worldInfo) != SOURCE_ABI_OK || worldInfo.plane_count != 18450 ||
					worldInfo.node_count != 5189 || worldInfo.leaf_count != 5353 ||
					worldInfo.cluster_count != 1081)
					return 37;
				SourceAbiWorldLeaf worldLeaf = {};
				if (bridge.WorldPointLeaf(0.0f, 0.0f, 0.0f, &worldLeaf) != SOURCE_ABI_OK ||
					worldLeaf.leaf_index >= worldInfo.leaf_count)
					return 38;
				uint64_t visibilityBytes = 0;
				if (bridge.WorldVisibility(0, SOURCE_WORLD_PVS, 0, 0,
					&visibilityBytes) != SOURCE_ABI_BUFFER_TOO_SMALL ||
					visibilityBytes != (worldInfo.cluster_count + 7) / 8)
					return 39;
				uint8_t visibility[8192];
				if (bridge.WorldVisibility(0, SOURCE_WORLD_PVS, visibility,
					sizeof(visibility), &visibilityBytes) != SOURCE_ABI_OK)
					return 40;
				const std::string miscVpk = root + "/hl2/hl2_misc_dir.vpk";
				if (bridge.ProbeVpk(miscVpk.c_str(), miscVpk.size(),
					&entryCount, &version) != SOURCE_ABI_OK || entryCount == 0 || version == 0)
					return 16;
				if (source_rust_bridge_read_path_add_vpk_flags(miscVpk.c_str(),
					miscVpk.size(), "GAME", 4, false, false) != SOURCE_ABI_OK)
					return 22;
				char cfg[1024];
				if (bridge.ReadFile("cfg/valve.rc", 12, "GAME", 4,
					cfg, sizeof(cfg), &required) != SOURCE_ABI_OK ||
					required == 0)
					return 17;
				uint64_t cfgSize = 0;
				if (source_rust_bridge_file_size("cfg/valve.rc", 12, "GAME", 4,
					&cfgSize) != SOURCE_ABI_OK || cfgSize != required)
					return 64;
				char resolvedReadPath[4096] = {};
				uint64_t resolvedReadBytes = 0;
				uint32_t resolvedReadKind = UINT32_MAX;
				// The shipped corpus keeps valve.rc packed, so the resolver has
				// to report the VPK mount and name the archive the entry came
				// from, which is the convention the native filesystem uses.
				const SourceAbiStatus valveResolveStatus =
					source_rust_bridge_resolve_read_path("cfg/valve.rc", 12,
						"GAME", 4, resolvedReadPath, sizeof(resolvedReadPath),
						&resolvedReadBytes, &resolvedReadKind);
				if (valveResolveStatus != SOURCE_ABI_OK ||
					resolvedReadKind != SOURCE_READ_PATH_VPK || resolvedReadBytes == 0 ||
					std::string(resolvedReadPath, resolvedReadBytes).find(
						"hl2_misc.vpk/cfg/valve.rc") == std::string::npos)
				{
					std::fprintf(stderr,
						"resolve cfg/valve.rc: status %d kind %u bytes %llu path %.*s\n",
						valveResolveStatus, resolvedReadKind,
						static_cast<unsigned long long>(resolvedReadBytes),
						static_cast<int>(resolvedReadBytes), resolvedReadPath);
					return 129;
				}
				char tinyResolvedRead[1] = {};
				if (source_rust_bridge_resolve_read_path("cfg/valve.rc", 12,
					"GAME", 4, tinyResolvedRead, sizeof(tinyResolvedRead),
					&resolvedReadBytes, &resolvedReadKind) != SOURCE_ABI_BUFFER_TOO_SMALL ||
					resolvedReadKind != SOURCE_READ_PATH_VPK || resolvedReadBytes == 0)
					return 130;
				// A file that really is loose still reports the disk mount.
				if (source_rust_bridge_resolve_read_path("cfg/config.cfg", 14,
					"GAME", 4, resolvedReadPath, sizeof(resolvedReadPath),
					&resolvedReadBytes, &resolvedReadKind) != SOURCE_ABI_OK ||
					resolvedReadKind != SOURCE_READ_PATH_DISK || resolvedReadBytes == 0 ||
					std::string(resolvedReadPath, resolvedReadBytes).find("/hl2/cfg/config.cfg") ==
						std::string::npos)
					return 131;
				uint32_t isDirectory = 0;
				if (source_rust_bridge_path_is_directory("cfg", 3, "GAME", 4,
					&isDirectory) != SOURCE_ABI_OK || isDirectory != 1)
					return 121;
				if (source_rust_bridge_path_is_directory("cfg/valve.rc", 12, "GAME", 4,
					&isDirectory) != SOURCE_ABI_OK || isDirectory != 0 ||
					source_rust_bridge_path_is_directory("missing-directory", 17, "GAME", 4,
						&isDirectory) != SOURCE_ABI_OK || isDirectory != 0)
					return 122;
				char findName[1024] = {};
				uint64_t findNameBytes = 0;
				uint64_t rustFind = 0;
				if (source_rust_bridge_find_first("cfg", 3, "GAME", 4,
					findName, sizeof(findName), &findNameBytes, &isDirectory,
					&rustFind) != SOURCE_ABI_OK || rustFind == 0 || isDirectory != 1 ||
					findNameBytes != 3 || std::memcmp(findName, "cfg", 3) != 0)
					return 124;
				if (source_rust_bridge_find_next(rustFind, findName, sizeof(findName),
					&findNameBytes, &isDirectory) != SOURCE_ABI_NOT_FOUND ||
					source_rust_bridge_find_close(rustFind) != SOURCE_ABI_OK ||
					source_rust_bridge_find_close(rustFind) != SOURCE_ABI_NOT_FOUND)
					return 125;
				if (source_rust_bridge_find_first("cfg/*.rc", 8, "GAME", 4,
					findName, sizeof(findName), &findNameBytes, &isDirectory,
					&rustFind) != SOURCE_ABI_OK || rustFind == 0)
					return 126;
				bool foundValveRc = false;
				SourceAbiStatus findStatus = SOURCE_ABI_OK;
				do
				{
					if (isDirectory != 0)
						return 127;
					if (findNameBytes == 8 && std::memcmp(findName, "valve.rc", 8) == 0)
						foundValveRc = true;
					findStatus = source_rust_bridge_find_next(rustFind, findName,
						sizeof(findName), &findNameBytes, &isDirectory);
				}
				while (findStatus == SOURCE_ABI_OK);
				if (!foundValveRc || findStatus != SOURCE_ABI_NOT_FOUND ||
					source_rust_bridge_find_close(rustFind) != SOURCE_ABI_OK)
					return 128;
				uint64_t rustReadFile = 0;
				uint64_t rustReadSize = 0;
				if (source_rust_bridge_file_open_read("cfg/valve.rc", 12, "GAME", 4,
					&rustReadFile, &rustReadSize) != SOURCE_ABI_OK || rustReadFile == 0 ||
					rustReadSize != cfgSize)
					return 116;
				char cursorRead[16] = {};
				uint64_t cursorBytes = 0;
				if (source_rust_bridge_file_read(rustReadFile, cursorRead,
					sizeof(cursorRead), &cursorBytes) != SOURCE_ABI_OK ||
					cursorBytes != sizeof(cursorRead) ||
					std::memcmp(cursorRead, cfg, sizeof(cursorRead)) != 0 ||
					source_rust_bridge_file_tell(rustReadFile, &cursorBytes) != SOURCE_ABI_OK ||
					cursorBytes != sizeof(cursorRead))
					return 117;
				if (source_rust_bridge_file_seek(rustReadFile, 0, 0,
					&cursorBytes) != SOURCE_ABI_OK || cursorBytes != 0 ||
					source_rust_bridge_open_file_size(rustReadFile,
						&rustReadSize) != SOURCE_ABI_OK || rustReadSize != cfgSize)
					return 118;
				uint64_t rejectedWrite = 99;
				if (source_rust_bridge_file_write(rustReadFile, "x", 1,
					&rejectedWrite) != SOURCE_ABI_IO_ERROR || rejectedWrite != 0 ||
					source_rust_bridge_file_seek(rustReadFile, 0, 2,
						&cursorBytes) != SOURCE_ABI_OK || cursorBytes != cfgSize ||
					source_rust_bridge_file_read(rustReadFile, cursorRead,
						sizeof(cursorRead), &cursorBytes) != SOURCE_ABI_OK || cursorBytes != 0)
					return 119;
				if (source_rust_bridge_file_close(rustReadFile) != SOURCE_ABI_OK ||
					source_rust_bridge_file_read(rustReadFile, cursorRead,
						sizeof(cursorRead), &cursorBytes) != SOURCE_ABI_NOT_FOUND ||
					cursorBytes != 0)
					return 120;
				if (source_rust_bridge_file_open_read("cfg/valve.rc", 12, 0, 0,
					&rustReadFile, &rustReadSize) != SOURCE_ABI_OK || rustReadFile == 0 ||
					rustReadSize != cfgSize ||
					source_rust_bridge_file_close(rustReadFile) != SOURCE_ABI_OK)
					return 123;

				const std::string sceneVpk = root + "/hl2/hl2_pak_dir.vpk";
				if (source_rust_bridge_read_path_add_vpk_flags(sceneVpk.c_str(),
					sceneVpk.size(), "GAME", 4, false, false) != SOURCE_ABI_OK)
					return 23;
				if (bridge.ProbeScene("scenes/scenes.image", 19, "GAME", 4,
					&sceneCount, &stringCount) != SOURCE_ABI_OK ||
					sceneCount == 0 || stringCount == 0)
					return 18;
				if (bridge.WorldClear() != SOURCE_ABI_OK ||
					bridge.WorldPointLeaf(0.0f, 0.0f, 0.0f, &worldLeaf) != SOURCE_ABI_NOT_FOUND)
					return 41;
				std::cout << "C++ ABI corpus smoke: " << entryCount << " VPK entries, "
					<< required << " cfg bytes, " << sceneCount << " scenes, "
					<< stringCount << " strings, " << worldInfo.leaf_count << " BSP leaves, "
					<< worldInfo.cluster_count << " visibility clusters passed\n";
			}

			if (bridge.HostTransition(SOURCE_HOST_CONTENT_READY) != SOURCE_ABI_OK ||
				bridge.HostTransition(SOURCE_HOST_LEGACY_RUNNING) != SOURCE_ABI_OK)
				return 36;
			static const char firstMap[] = "d1_trainstation_01";
			static const char nextMap[] = "d1_trainstation_02";
			static const char landmark[] = "rust_landmark";
			if (bridge.HostRequestOperation(SOURCE_HOST_OPERATION_NEW_GAME,
				firstMap, sizeof(firstMap) - 1, "", 0, 0) != SOURCE_ABI_OK)
				return 56;
			uint32_t operationKind = 0;
			if (bridge.HostPendingOperation(&operationKind) != SOURCE_ABI_OK ||
				operationKind != SOURCE_HOST_OPERATION_NEW_GAME)
				return 57;
			if (bridge.HostRequestOperation(SOURCE_HOST_OPERATION_CHANGE_LEVEL_SP,
				nextMap, sizeof(nextMap) - 1, landmark, sizeof(landmark) - 1, 0) != SOURCE_ABI_OK)
				return 58;
			SourceAbiHostOperationInfo operationInfo = {};
			if (bridge.HostTakeOperation(0, 0, 0, 0, &operationInfo) !=
				SOURCE_ABI_BUFFER_TOO_SMALL ||
				operationInfo.target_length != sizeof(nextMap) - 1 ||
				operationInfo.landmark_length != sizeof(landmark) - 1)
				return 59;
			char operationTarget[64] = {};
			char operationLandmark[64] = {};
			if (bridge.HostTakeOperation(operationTarget, sizeof(operationTarget),
				operationLandmark, sizeof(operationLandmark), &operationInfo) != SOURCE_ABI_OK ||
				operationInfo.kind != SOURCE_HOST_OPERATION_CHANGE_LEVEL_SP ||
				std::memcmp(operationTarget, nextMap, sizeof(nextMap) - 1) != 0 ||
				std::memcmp(operationLandmark, landmark, sizeof(landmark) - 1) != 0)
				return 60;
			if (bridge.HostPendingOperation(&operationKind) != SOURCE_ABI_NOT_FOUND ||
				bridge.HostClearOperation() != SOURCE_ABI_OK)
				return 61;
			SourceAbiTickPlan tickPlan = {};
			if (bridge.HostScheduleTicks(0.010, 0.015, 100, true, false,
				&tickPlan) != SOURCE_ABI_OK || tickPlan.tick_count != 0 ||
				tickPlan.remainder < 0.009999 || tickPlan.remainder > 0.010001)
				return 62;
			if (bridge.HostScheduleTicks(0.020, 0.015, 100, true, true,
				&tickPlan) != SOURCE_ABI_OK || tickPlan.tick_count != 2 ||
				tickPlan.remainder < -0.000001 || tickPlan.remainder > 0.000001)
				return 63;
			uint32_t sessionCount = 0;
			if (bridge.HostRunSessions(SessionCallback, 0, 4, &sessionCount) != SOURCE_ABI_OK ||
				sessionCount != 3 || g_SessionCalls != 3)
				return 42;
			uint32_t nestedSessionCount = 0;
			if (bridge.HostRunSessions(NestedFrameSessionCallback, 0, 1,
				&nestedSessionCount) != SOURCE_ABI_OK || nestedSessionCount != 1 ||
				g_FrameCalls != 4)
				return 53;
			if (bridge.HostTransition(SOURCE_HOST_SHUTTING_DOWN) != SOURCE_ABI_OK ||
				bridge.HostTransition(SOURCE_HOST_STOPPED) != SOURCE_ABI_OK)
				return 36;
		}
		if (bridge.Shutdown() != SOURCE_ABI_OK)
			return 7;
		if (source_rust_bridge_world_clear() != SOURCE_ABI_INVALID_ARGUMENT)
			return 44;
	}

	if (source_context_live_count() != 0 || g_LogCalls != 1)
		return 8;
	std::cout << "C++ ABI smoke: 10000 lifecycle cycles passed\n";
	return 0;
}
