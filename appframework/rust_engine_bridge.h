#ifndef APPFRAMEWORK_RUST_ENGINE_BRIDGE_H
#define APPFRAMEWORK_RUST_ENGINE_BRIDGE_H

#include "rust/source_abi.h"

#if defined(_WIN32)
# if defined(SOURCE_RUST_BRIDGE_BUILD)
#  define SOURCE_RUST_BRIDGE_EXPORT __declspec(dllexport)
# else
#  define SOURCE_RUST_BRIDGE_EXPORT __declspec(dllimport)
# endif
#else
# define SOURCE_RUST_BRIDGE_EXPORT __attribute__((visibility("default")))
#endif

// Process-wide access for legacy modules that are loaded after the launcher.
// The active value is still an opaque C ABI handle; no Rust or C++ object
// ownership crosses the module boundary.
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_activate(
	SourceAbiHandle handle);
extern "C" SOURCE_RUST_BRIDGE_EXPORT void source_rust_bridge_deactivate(
	SourceAbiHandle handle);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_world_load(
	const char *virtualPath, uint64_t virtualPathLength, SourceAbiWorldInfo *info);
// The Metal presenter attached to the game window, published by the window
// manager and read by the engine's frame loop. Zero when the window is not
// Metal-backed, which is every configuration but -metal.
extern "C" SOURCE_RUST_BRIDGE_EXPORT void source_rust_bridge_set_presenter(
	SourceAbiHandle presenter);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiHandle source_rust_bridge_presenter();
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_scene_load(
	const char *map, uint64_t mapLength, SourceAbiWorldDraw *drawn);
// The engine's two-dimensional output, forwarded to the Rust renderer so
// the interface is composited over the world it draws. Each is a no-op
// returning failure when no Metal presenter is attached, which is every
// configuration that still reaches the screen another way.
extern "C" SOURCE_RUST_BRIDGE_EXPORT bool source_rust_bridge_ui_active();
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_ui_begin();
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_ui_texture(
	uint32_t id, uint32_t width, uint32_t height, const uint8_t *rgba );
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_ui_texture_region(
	uint32_t id, uint32_t x, uint32_t y, uint32_t width, uint32_t height,
	const uint8_t *rgba );
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_ui_texture_alias(
	uint32_t alias, uint32_t base );
extern "C" SOURCE_RUST_BRIDGE_EXPORT bool source_rust_bridge_ui_has_texture( uint32_t id );
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_ui_quad(
	uint32_t texture, const float *bounds, const float *coords, const float *tint );
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_ui_end(
	uint32_t width, uint32_t height, uint64_t *quads );

extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_scene_present(
	const float *position, const float *angles, SourceAbiWorldDraw *drawn);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_world_clear();
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_read_file(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, void *output, uint64_t outputLength, uint64_t *written);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_size(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, uint64_t *size);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_resolve_read_path(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, void *output, uint64_t outputLength, uint64_t *written,
	uint32_t *kind);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_path_is_directory(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, uint32_t *isDirectory);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_find_first(
	const char *wildcard, uint64_t wildcardLength, const char *pathId,
	uint64_t pathIdLength, void *output, uint64_t outputLength, uint64_t *written,
	uint32_t *isDirectory, uint64_t *find);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_find_next(
	uint64_t find, void *output, uint64_t outputLength, uint64_t *written,
	uint32_t *isDirectory);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_find_close(
	uint64_t find);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_read_paths_clear();
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_read_path_add_directory_flags(
	const char *root, uint64_t rootLength, const char *pathId,
	uint64_t pathIdLength, bool atHead, bool byRequestOnly, bool allowSymlinkEscape);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_read_path_add_vpk_flags(
	const char *directoryPath, uint64_t directoryPathLength, const char *pathId,
	uint64_t pathIdLength, bool atHead, bool byRequestOnly);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_write_paths_clear();
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_write_path_add(
	const char *root, uint64_t rootLength, const char *pathId,
	uint64_t pathIdLength, bool atHead);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_write_path_add_flags(
	const char *root, uint64_t rootLength, const char *pathId,
	uint64_t pathIdLength, bool atHead, bool byRequestOnly);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_resolve_write_path(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, void *output, uint64_t outputLength, uint64_t *written);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_create_write_directory(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_remove_write_file(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_rename_write_file(
	const char *oldVirtualPath, uint64_t oldVirtualPathLength, const char *oldPathId,
	uint64_t oldPathIdLength, const char *newVirtualPath, uint64_t newVirtualPathLength,
	const char *newPathId, uint64_t newPathIdLength);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_is_write_file_writable(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, uint32_t *writable);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_set_write_file_writable(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, bool writable);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_open_read(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, uint64_t *file, uint64_t *size);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_open_write(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, const char *mode, uint64_t modeLength,
	uint64_t *file, uint64_t *size);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_close(
	uint64_t file);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_read(
	uint64_t file, void *output, uint64_t outputLength, uint64_t *read);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_write(
	uint64_t file, const void *input, uint64_t inputLength, uint64_t *written);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_flush(
	uint64_t file);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_seek(
	uint64_t file, int64_t offset, uint32_t origin, uint64_t *position);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_tell(
	uint64_t file, uint64_t *position);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_open_file_size(
	uint64_t file, uint64_t *size);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_file_is_open(
	uint64_t file, uint32_t *open);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_channel_create(
	int32_t outgoingSequence, int32_t incomingSequence, int32_t outgoingAck,
	uint64_t *channelId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_channel_remove(
	uint64_t channelId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_channel_reset(
	uint64_t channelId, int32_t outgoingSequence, int32_t incomingSequence,
	int32_t outgoingAck);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_channel_preview_incoming(
	uint64_t channelId, int32_t sequence, int32_t outgoingAck, uint32_t choked,
	int32_t maxDrop, SourceAbiNetPacketDecision *decision);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_channel_commit_incoming(
	uint64_t channelId, int32_t sequence, int32_t outgoingAck, uint32_t choked,
	int32_t maxDrop, SourceAbiNetPacketDecision *decision);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_channel_advance_outgoing(
	uint64_t channelId, SourceAbiNetSequenceAdvance *advance);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_packet_checksum(
	const void *payload, uint64_t payloadLength, uint16_t *checksum);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_packet_encode_header(
	int32_t sequence, int32_t outgoingAck, uint32_t reliableState,
	bool hasChoked, uint32_t choked, bool hasChallenge, uint32_t challenge,
	bool checksumRequired, SourceAbiNetEncodedPacketHeader *header);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_packet_finalize_header(
	void *packet, uint64_t packetLength, uint32_t flags, bool checksumRequired,
	uint16_t *checksum);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_net_packet_header(
	const void *packet, uint64_t packetLength, bool checksumRequired,
	bool expectsChallenge, uint32_t expectedChallenge, SourceAbiNetPacketHeader *header);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_lzss_compress(
	const void *input, uint64_t inputLength, uint32_t window, void *output,
	uint64_t capacity, uint64_t *length);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_lzss_decompress(
	const void *input, uint64_t inputLength, void *output, uint64_t capacity,
	uint64_t *length);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_lzss_actual_size(
	const void *input, uint64_t inputLength, uint64_t *actualSize);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_split_packet_header_encode(
	int32_t sequence, uint32_t packetNumber, uint32_t packetCount, uint32_t splitSize,
	void *bytes);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_split_packet_header_decode(
	const void *datagram, uint64_t datagramLength, SourceAbiSplitPacketHeader *header);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_split_packet_create(
	uint64_t *peerId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_split_packet_remove(
	uint64_t peerId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_split_packet_reset(
	uint64_t peerId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_split_packet_accept(
	uint64_t peerId, const void *datagram, uint64_t datagramLength, void *message,
	uint64_t capacity, uint64_t *length, bool *complete);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_split_packet_collect(
	uint64_t peerId, void *message, uint64_t capacity, uint64_t *length);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_create(
	const char *name, uint64_t nameLength, uint32_t maxEntries, int32_t tick,
	uint64_t *tableId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_remove(
	uint64_t tableId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_clear(
	uint64_t tableId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_enable_history(
	uint64_t tableId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_set_tick(
	uint64_t tableId, int32_t tick);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_synchronize_tick(
	uint64_t tableId, int32_t tick);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_upsert(
	uint64_t tableId, const char *value, uint64_t valueLength, bool updateUserData,
	const void *userData, uint64_t userDataLength, SourceAbiStringTableUpsert *result);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_set_user_data(
	uint64_t tableId, uint32_t index, const void *userData, uint64_t userDataLength,
	SourceAbiStringTableChange *change);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_find(
	uint64_t tableId, const char *value, uint64_t valueLength, uint32_t *index);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_changed_since(
	uint64_t tableId, int32_t tick, uint32_t *changed);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_restore_tick(
	uint64_t tableId, int32_t tick, int32_t *lastChangedTick);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_entry_count(
	uint64_t tableId, uint32_t *entryCount);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_encode_entries(
	uint64_t tableId, uint8_t *outBytes, uint64_t capacity,
	uint32_t *outBitCount);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_read_string(
	uint64_t tableId, uint32_t index, void *output, uint64_t outputLength,
	uint64_t *written);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_string_table_read_user_data(
	uint64_t tableId, uint32_t index, void *output, uint64_t outputLength,
	uint64_t *written);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_data_table_clear();
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_data_table_register(
	const char *name, uint64_t nameLength, uint32_t propertyCount,
	SourceAbiDataTableRegistration *registration);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_data_table_register_property(
	uint32_t tableId, const SourceAbiDataTableProperty *property);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_server_class_register(
	const char *name, uint64_t nameLength, uint32_t tableId, uint32_t *classId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_data_table_finalize(
	uint32_t nativeCompatibilityCrc, SourceAbiDataTableSummary *summary);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_data_table_summary(
	SourceAbiDataTableSummary *summary);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_server_class_find(
	const char *name, uint64_t nameLength, uint32_t *classId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_snapshot_clear();
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_snapshot_queue_delete(
	uint32_t slot, uint32_t *queued);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_snapshot_create(
	int32_t tick, uint32_t maxEntities, const SourceAbiSnapshotEntity *entities,
	uint64_t entityCount, uint64_t *snapshotId, SourceAbiSnapshotSummary *summary);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_snapshot_remove(
	uint64_t snapshotId);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_snapshot_summary(
	uint64_t snapshotId, SourceAbiSnapshotSummary *summary);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_snapshot_entity_at(
	uint64_t snapshotId, uint32_t ordinal, SourceAbiSnapshotEntity *entity);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_snapshot_delete_at(
	uint64_t snapshotId, uint32_t ordinal, uint32_t *slot);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_snapshot_delta(
	uint64_t fromSnapshotId, const uint32_t *fromVisible, uint64_t fromVisibleCount,
	uint64_t toSnapshotId, const uint32_t *toVisible, uint64_t toVisibleCount,
	SourceAbiSnapshotDelta *deltas, uint64_t capacity, uint64_t *count);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_demo_header_encode(
	int32_t demoProtocol, int32_t networkProtocol, const char *serverName,
	const char *clientName, const char *mapName, const char *gameDirectory,
	float playbackTime, int32_t playbackTicks, int32_t playbackFrames,
	int32_t signonLength, uint8_t *bytes, uint64_t capacity, uint64_t *size);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_delta_header_encode(
	uint32_t entityIndex, int32_t headerBase, bool leavePvs, bool deleteEntity,
	bool enterPvs, uint8_t *bytes, uint64_t capacity, uint32_t *bitCount);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_command_enqueue(
	const char *script, uint64_t scriptLength, uint32_t *commandCount);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_command_pop(
	void *output, uint64_t outputLength, uint64_t *written);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_demo_validate(
	const char *virtualPath, uint64_t virtualPathLength, SourceAbiDemoInfo *info);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_save_validate(
	const char *virtualPath, uint64_t virtualPathLength, SourceAbiSaveInfo *info);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_save_container_header_encode(
	const void *tokens, uint64_t tokensLength, uint64_t tokenCount,
	const void *gameData, uint64_t gameDataLength, void *outBytes,
	uint64_t capacity, uint64_t *outSize);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_save_map_state_encode(
	const void *tokens, uint64_t tokensLength, uint64_t tokenCount,
	const void *dataHeaders, uint64_t dataHeadersLength, const void *data,
	uint64_t dataLength, void *outBytes, uint64_t capacity, uint64_t *outSize);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_host_run_frames(
	SourceAbiFrameFn runFrame, void *userData, uint64_t maxIterations,
	SourceAbiFrameLoopInfo *info);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_host_pace(
	uint64_t nowNs, uint64_t minimumFrameNs, SourceAbiFramePace *pace);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_host_schedule_ticks(
	double frameTime, double tickInterval, int32_t startTick, bool accumulate,
	bool alternateTicks, SourceAbiTickPlan *plan);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_host_request_operation(
	uint32_t kind, const char *target, uint64_t targetLength,
	const char *landmark, uint64_t landmarkLength, uint32_t flags);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_host_take_operation(
	void *targetOutput, uint64_t targetOutputLength, void *landmarkOutput,
	uint64_t landmarkOutputLength, SourceAbiHostOperationInfo *info);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_host_pending_operation(
	uint32_t *kind);
extern "C" SOURCE_RUST_BRIDGE_EXPORT SourceAbiStatus source_rust_bridge_host_clear_operation();

// Transitional C++ lifecycle owner. The C++ side retains only an opaque handle;
// buffers remain caller-owned and all implementation state remains in Rust.
class SOURCE_RUST_BRIDGE_EXPORT CRustEngineBridge
{
public:
	CRustEngineBridge();
	~CRustEngineBridge();

	SourceAbiStatus Init(SourceAbiLogFn log = 0, void *userData = 0);
	SourceAbiStatus Shutdown();
	SourceAbiStatus Echo(const void *input, uint64_t inputLength,
		void *output, uint64_t outputLength, uint64_t *written) const;
	SourceAbiStatus EmitLog(int32_t level, const char *message, uint64_t length) const;
	SourceAbiStatus ResolveContentPath(const char *root, uint64_t rootLength,
		const char *virtualPath, uint64_t virtualPathLength, void *output,
		uint64_t outputLength, uint64_t *written) const;
	SourceAbiStatus ExecutableBase(void *output, uint64_t outputLength,
		uint64_t *written) const;
	SourceAbiStatus MountDirectory(const char *root, uint64_t rootLength,
		const char *pathId, uint64_t pathIdLength, bool atHead) const;
	SourceAbiStatus MountVpk(const char *directoryPath, uint64_t directoryPathLength,
		const char *pathId, uint64_t pathIdLength, bool atHead) const;
	SourceAbiStatus ReadFile(const char *virtualPath, uint64_t virtualPathLength,
		const char *pathId, uint64_t pathIdLength, void *output,
		uint64_t outputLength, uint64_t *written) const;
	SourceAbiStatus ProbeScene(const char *virtualPath, uint64_t virtualPathLength,
		const char *pathId, uint64_t pathIdLength, uint64_t *sceneCount,
		uint64_t *stringCount) const;
	SourceAbiStatus DemoValidate(const char *virtualPath, uint64_t virtualPathLength,
		const char *pathId, uint64_t pathIdLength, SourceAbiDemoInfo *info) const;
	SourceAbiStatus SaveValidate(const char *virtualPath, uint64_t virtualPathLength,
		const char *pathId, uint64_t pathIdLength, SourceAbiSaveInfo *info) const;
	SourceAbiStatus WorldLoad(const char *virtualPath, uint64_t virtualPathLength,
		const char *pathId, uint64_t pathIdLength, SourceAbiWorldInfo *info) const;
	SourceAbiStatus WorldClear() const;
	SourceAbiStatus WorldPointLeaf(float x, float y, float z,
		SourceAbiWorldLeaf *leaf) const;
	SourceAbiStatus WorldVisibility(uint32_t cluster, uint32_t kind, void *output,
		uint64_t outputLength, uint64_t *written) const;
	SourceAbiStatus HostTransition(uint32_t phase) const;
	SourceAbiStatus HostPhase(uint32_t *phase) const;
	SourceAbiStatus HostConfigureScheduler(uint64_t tickIntervalNs,
		uint32_t maxCatchUpTicks) const;
	SourceAbiStatus HostAdvance(uint64_t nowNs, SourceAbiFramePlan *plan) const;
	SourceAbiStatus HostPace(uint64_t nowNs, uint64_t minimumFrameNs,
		SourceAbiFramePace *pace) const;
	SourceAbiStatus HostScheduleTicks(double frameTime, double tickInterval,
		int32_t startTick, bool accumulate, bool alternateTicks,
		SourceAbiTickPlan *plan) const;
	SourceAbiStatus HostRequestOperation(uint32_t kind, const char *target,
		uint64_t targetLength, const char *landmark, uint64_t landmarkLength,
		uint32_t flags) const;
	SourceAbiStatus HostTakeOperation(void *targetOutput, uint64_t targetOutputLength,
		void *landmarkOutput, uint64_t landmarkOutputLength,
		SourceAbiHostOperationInfo *info) const;
	SourceAbiStatus HostPendingOperation(uint32_t *kind) const;
	SourceAbiStatus HostClearOperation() const;
	SourceAbiStatus HostRunSessions(SourceAbiSessionFn runSession, void *userData,
		uint32_t maxSessions, uint32_t *sessionCount) const;
	SourceAbiStatus HostRunFrames(SourceAbiFrameFn runFrame, void *userData,
		uint64_t maxIterations, SourceAbiFrameLoopInfo *info) const;
	SourceAbiStatus CvarRegister(const char *name, uint64_t nameLength,
		const char *defaultValue, uint64_t defaultValueLength, uint32_t flags) const;
	SourceAbiStatus CvarSet(const char *name, uint64_t nameLength,
		const char *value, uint64_t valueLength) const;
	SourceAbiStatus CvarGet(const char *name, uint64_t nameLength, void *output,
		uint64_t outputLength, uint64_t *written, SourceAbiCvarInfo *info) const;
	SourceAbiStatus CommandEnqueue(const char *script, uint64_t scriptLength,
		uint32_t *commandCount) const;
	SourceAbiStatus CommandPop(void *output, uint64_t outputLength,
		uint64_t *written) const;
	SourceAbiStatus InputModifier(uint32_t modifier, bool pressed,
		uint32_t *modifierMask) const;
	SourceAbiStatus InputMouseButton(uint32_t button, bool pressed,
		uint32_t *buttonMask) const;
	SourceAbiStatus InputMouseMotion(int32_t deltaX, int32_t deltaY) const;
	SourceAbiStatus InputTakeMouseDelta(int32_t *deltaX, int32_t *deltaY) const;
	SourceAbiStatus InputReset() const;
	SourceAbiStatus ProbeVpk(const char *path, uint64_t pathLength,
		uint64_t *entryCount, uint32_t *version) const;
	SourceAbiStatus ReadVpkFile(const char *directoryPath, uint64_t directoryPathLength,
		const char *entryPath, uint64_t entryPathLength, void *output,
		uint64_t outputLength, uint64_t *written) const;
	SourceAbiStatus ProbeVpkScene(const char *directoryPath, uint64_t directoryPathLength,
		const char *entryPath, uint64_t entryPathLength, uint64_t *sceneCount,
		uint64_t *stringCount) const;

	bool IsInitialized() const { return m_Handle != 0; }
	SourceAbiHandle Handle() const { return m_Handle; }
	SourceAbiHandle HandleForTesting() const { return m_Handle; }

private:
	CRustEngineBridge(const CRustEngineBridge &);
	CRustEngineBridge &operator=(const CRustEngineBridge &);

	SourceAbiHandle m_Handle;
};

#endif
