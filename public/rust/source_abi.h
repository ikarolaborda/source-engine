#ifndef SOURCE_RUST_SOURCE_ABI_H
#define SOURCE_RUST_SOURCE_ABI_H

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32)
# if defined(SOURCE_ABI_BUILD)
#  define SOURCE_ABI_EXPORT __declspec(dllexport)
# else
#  define SOURCE_ABI_EXPORT __declspec(dllimport)
# endif
#else
# define SOURCE_ABI_EXPORT __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

#define SOURCE_ABI_VERSION 1u

typedef uint64_t SourceAbiHandle;

typedef struct SourceAbiSlice {
	const uint8_t *data;
	uint64_t length;
} SourceAbiSlice;

typedef struct SourceAbiMutSlice {
	uint8_t *data;
	uint64_t length;
} SourceAbiMutSlice;

typedef int32_t SourceAbiStatus;

/* Metadata-only ZIP/BSP index. Offsets are relative to the ZIP; input bytes are
 * borrowed only during create. Stored/LZMA payload decoding/CRC is separate.
 * The reserved __preload_section.pre cache entry is hidden. Call destroy before
 * unloading the ABI library. Queries may run concurrently; no native pointer
 * is retained. The native adapter and Rust both bound archives to 512 MiB. */
#define SOURCE_PAK_INDEX_MAX_ARCHIVE_BYTES (512u * 1024u * 1024u)
typedef struct SourceAbiPakEntry {
	uint64_t data_offset;
	uint32_t length, compressed_length, method, index, crc32, reserved;
} SourceAbiPakEntry;
SOURCE_ABI_EXPORT SourceAbiStatus source_pak_index_create(
	SourceAbiSlice input, uint64_t *out_handle, uint32_t *out_count);
/* Read-only UTF-8 regular-file path and ZIP byte range. Rust retains the file
 * descriptor and metadata but releases the temporary buffer. Payload opens use
 * that descriptor, validating size/CRC (not an atomic file snapshot guarantee).
 * Invalid ranges return IO_ERROR; malformed/oversized ZIPs return FORMAT_ERROR.
 * Outputs are zeroed on failure when
 * both are provided. An in-bounds zero-byte range is valid (empty BSP lump).
 * No native read callback or parser fallback is used. */
SOURCE_ABI_EXPORT SourceAbiStatus source_pak_index_open_file(
	SourceAbiSlice path, uint64_t offset, uint64_t length,
	uint64_t *out_handle, uint32_t *out_count);
typedef struct SourceAbiPakArchiveInfo {
	uint64_t offset, length;
	int64_t modified_seconds;
	uint32_t entry_count, reserved;
} SourceAbiPakArchiveInfo;
/* kind 0 = standalone ZIP, 1 = BSP embedded ZIP (versions 19..21).
 * Rust discovers the range and parses the index on one retained descriptor.
 * Empty BSP lump returns NOT_FOUND; malformed/truncated headers/ranges return
 * FORMAT_ERROR. Other map lumps are not validated here. Outputs are zero on
 * failure when both are provided. Timestamp is Unix seconds (0 if unavailable). */
SOURCE_ABI_EXPORT SourceAbiStatus source_pak_index_open_archive(
	SourceAbiSlice path, uint32_t kind, uint64_t *out_handle,
	SourceAbiPakArchiveInfo *out_info);
/* Fully validate/decode an entry into an independent context-owned read-only
 * file. Use the existing context file read/seek/tell/size/close APIs. Seeks
 * clamp to [0,size]; invalid origins fail without moving. Index destruction
 * does not close opened files. Slice-created indexes have no payload source
 * and return INVALID_ARGUMENT. No output handle on failed IO/decoding/CRC. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_open_pak(
	uint64_t context, uint64_t index, uint32_t entry,
	uint64_t *out_file, uint64_t *out_size, uint64_t *out_absolute_offset);
SOURCE_ABI_EXPORT SourceAbiStatus source_pak_index_destroy(uint64_t handle);
/* Context-independent ordered mount ownership. Native resource contexts remain
 * opaque. Callbacks must not unwind and must support the invoking thread;
 * callback code/data must outlive every table/snapshot, including module unload.
 * insert transfers ownership only on OK; failures leave it with the caller.
 * get borrows until removal: caller serializes native borrows against mutation.
 * snapshot clones native headers/resources; NULL aborts and releases prior copies.
 * All drop/clone callbacks run outside locks and may reenter these APIs.
 * remove fast=0 preserves order; fast=1 swaps in the last entry. Max65536 mounts.
 * Failed handle/count/get outputs are zero/NULL; index miss is NOT_FOUND. */
typedef void (*SourceAbiMountDropFn)(void *resource);
typedef void *(*SourceAbiMountCloneFn)(const void *resource);
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_table_create(SourceAbiMountDropFn dropFn, SourceAbiMountCloneFn cloneFn, uint64_t *handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_table_insert(uint64_t handle, uint32_t index, void *resource);
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_table_count(uint64_t handle, uint32_t *count);
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_table_get(uint64_t handle, uint32_t index, void **resource);
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_table_remove(uint64_t handle, uint32_t index, uint8_t fast);
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_table_clear(uint64_t handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_table_snapshot(uint64_t handle, uint64_t *copy);
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_table_destroy(uint64_t handle);
/* Monotonic positive store IDs; returns INTERNAL_ERROR instead of wrapping. */
SOURCE_ABI_EXPORT SourceAbiStatus source_mount_store_id_next(int32_t *out_id);

typedef struct SourceAbiSearchPath {
	SourceAbiSlice path_id;
	int32_t store_id;
	uint32_t flags;
} SourceAbiSearchPath;
enum {
	SOURCE_SEARCH_PACK = 1, SOURCE_SEARCH_MAP = 2,
	SOURCE_SEARCH_BY_REQUEST = 4, SOURCE_SEARCH_EXCLUDED = 8
};
/* Context-independent immutable search plan. Copies only selected source
 * indices, not pointers/resources. Up to 65536 paths, UTF-8 IDs up to 4096 bytes.
 * Filter 0=all, 1=cull ZIP/BSP, 2=cull non-ZIP/BSP; VPK is non-pack here.
 * Filtering precedes store-ID deduplication. MAP requires PACK; no unknown bits.
 * has_requested is 0/1; absent requires an empty slice. Empty explicit ID is valid.
 * Failed creation zeros out_handle. The caller retains its resource snapshot. */
SOURCE_ABI_EXPORT SourceAbiStatus source_search_plan_create(
	const SourceAbiSearchPath *paths, uint32_t count, SourceAbiSlice requested,
	uint8_t has_requested, uint32_t filter, uint64_t *out_handle);
/* Exhaustion is NOT_FOUND and out_index=UINT32_MAX; invalid calls don't advance. */
SOURCE_ABI_EXPORT SourceAbiStatus source_search_plan_next(uint64_t handle, uint32_t *out_index);
SOURCE_ABI_EXPORT SourceAbiStatus source_search_visits_create(uint64_t *out_handle);
/* Only a visits handle: out_seen=1 for a duplicate, 0 for new, 1 on error. */
SOURCE_ABI_EXPORT SourceAbiStatus source_search_visits_mark(uint64_t handle, int32_t store, uint32_t *out_seen);
/* Reset rewinds a plan or clears visits. Destroy exactly once; stale => INVALID_HANDLE. */
SOURCE_ABI_EXPORT SourceAbiStatus source_search_state_reset(uint64_t handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_search_state_destroy(uint64_t handle);
/* Context-free native/Rust read-path selection. Flags must be 0/1. With no
 * requested ID, only non-by-request paths match. Explicit IDs ignore that flag;
 * BSP matches only GAME map packs. Comparisons are ASCII case-insensitive.
 * Empty IDs are allowed; has_requested distinguishes an explicit empty ID from
 * no ID (which requires a zero-length requested slice). Invalid input zeros the
 * provided result and returns INVALID_ARGUMENT. No IO or native pointers kept. */
SOURCE_ABI_EXPORT SourceAbiStatus source_read_path_matches(
	SourceAbiSlice stored, SourceAbiSlice requested, uint8_t has_requested,
	uint8_t by_request_only, uint8_t is_map_pack, uint32_t *out_matches);
/* Native fixed-field enumeration: omit names exceeding max_name_bytes (not
 * including NUL), which must be nonzero. The separate output buffer still uses
 * BUFFER_TOO_SMALL without publishing/advancing a cursor. No truncation.
 * Close/advance via context find APIs. The unrestricted find_first is unchanged. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_find_first_bounded(
	uint64_t context, SourceAbiSlice wildcard, SourceAbiSlice path_id, uint32_t max_name_bytes,
	SourceAbiMutSlice output, uint64_t *out_written, uint32_t *out_is_directory, uint64_t *out_find);
/* Mount a shared file-backed index: no reopen/reparse, no native-supplied range.
 * The mount retains the archive after index-handle destruction. Clearing mounts
 * does not close independent indexes or decoded files. Metadata-only indexes
 * return INVALID_ARGUMENT. Flags must be 0 or 1; path ID is nonempty UTF-8. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_read_path_add_pak_index(
	uint64_t context, uint64_t index, SourceAbiSlice path_id,
	uint8_t at_head, uint8_t by_request_only);
/* Snapshot numbered ZIP paths in mount-precedence order: descending numbers,
 * localized Xbox360 series before base. naming: 0 desktop, 1 Xbox360; empty
 * language disables localization. Each series stops at the first stat failure;
 * present malformed archives remain candidates. At most 65536 per series,
 * otherwise IO_ERROR, no partial snapshot. Full paths (no NUL) retain root
 * spelling. Empty result => NOT_FOUND. Short first buffer publishes no cursor.
 * Consume/close with context find_next/find_close; is_directory is always 0.
 * Snapshot owns names only; files may change before they are opened. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_find_pack_candidates(
	uint64_t context, SourceAbiSlice root, uint32_t naming, SourceAbiSlice language,
	SourceAbiMutSlice output, uint64_t *out_written, uint32_t *out_is_directory,
	uint64_t *out_find);
/* Archive-local glob using the ordinary Rust ASCII-case-insensitive bytewise
 * wildcard policy (*, ?, *.*).
 * Returns canonical relative paths, sorted files then deduplicated implied
 * directories. A file wins a colliding directory name. Dot/slash normalization
 * is allowed within the root; escape, absolute paths and wildcard directories
 * are rejected. No matching entries => NOT_FOUND. Short output reports size
 * without publishing a cursor. Use context find_next/find_close; the cursor
 * survives index destruction and does not retain a native pointer. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_find_first_pak(
	uint64_t context, uint64_t index, SourceAbiSlice pattern, SourceAbiMutSlice output,
	uint64_t *out_written, uint32_t *out_is_directory, uint64_t *out_find);
SOURCE_ABI_EXPORT SourceAbiStatus source_pak_index_find(
	uint64_t handle, SourceAbiSlice path, SourceAbiPakEntry *out_entry);
/* Canonical name has no NUL. BUFFER_TOO_SMALL reports required length and
 * metadata without writing name bytes. Out-of-range index returns NOT_FOUND. */
SOURCE_ABI_EXPORT SourceAbiStatus source_pak_index_entry(
	uint64_t handle, uint32_t index, SourceAbiPakEntry *out_entry,
	uint8_t *name, uint64_t capacity, uint64_t *written);
enum {
	SOURCE_ABI_OK = 0,
	SOURCE_ABI_INVALID_ARGUMENT = 1,
	SOURCE_ABI_UNSUPPORTED_VERSION = 2,
	SOURCE_ABI_INVALID_HANDLE = 3,
	SOURCE_ABI_BUFFER_TOO_SMALL = 4,
	SOURCE_ABI_PANIC = 5,
	SOURCE_ABI_INTERNAL_ERROR = 6,
	SOURCE_ABI_IO_ERROR = 7,
	SOURCE_ABI_FORMAT_ERROR = 8,
	SOURCE_ABI_NOT_FOUND = 9,
	/* The operation declined rather than failed: compressing a buffer that
	 * did not get smaller, where the caller stores the original instead. */
	SOURCE_ABI_DECLINED = 10
};

enum {
	SOURCE_READ_PATH_DISK = 0,
	SOURCE_READ_PATH_VPK = 1,
	/* From the archive embedded in the loaded map. */
	SOURCE_READ_PATH_PAK = 2
};

enum {
	SOURCE_INPUT_MOD_CAPS_LOCK = 1u << 0,
	SOURCE_INPUT_MOD_RIGHT_SHIFT = 1u << 1,
	SOURCE_INPUT_MOD_LEFT_SHIFT = 1u << 2,
	SOURCE_INPUT_MOD_RIGHT_CONTROL = 1u << 3,
	SOURCE_INPUT_MOD_LEFT_CONTROL = 1u << 4,
	SOURCE_INPUT_MOD_RIGHT_ALT = 1u << 5,
	SOURCE_INPUT_MOD_LEFT_ALT = 1u << 6,
	SOURCE_INPUT_MOD_RIGHT_GUI = 1u << 7,
	SOURCE_INPUT_MOD_LEFT_GUI = 1u << 8
};

typedef void (*SourceAbiLogFn)(void *user_data, int32_t level, SourceAbiSlice message);
typedef int32_t (*SourceAbiSessionFn)(void *user_data);
typedef int32_t (*SourceAbiFrameFn)(void *user_data);
typedef int32_t (*SourceAbiAppSystemFn)(void *user_data, uint32_t operation, uint32_t index);

enum {
	SOURCE_APP_CREATE = 0, SOURCE_APP_CONNECT = 1, SOURCE_APP_PREINIT = 2,
	SOURCE_APP_INIT = 3, SOURCE_APP_MAIN = 4, SOURCE_APP_SHUTDOWN = 5,
	SOURCE_APP_POSTSHUTDOWN = 6, SOURCE_APP_DISCONNECT = 7,
	SOURCE_APP_REMOVE_SYSTEMS = 8, SOURCE_APP_UNLOAD_MODULES = 9, SOURCE_APP_DESTROY = 10
};

/* Create returns system count or negative failure. Connect/PreInit/Init return
 * zero on success; failed callbacks undo their own partial work. Main returns
 * its exit code. Cleanup callbacks cannot fail. No pointer is retained, no
 * lock spans a callback, and nested groups are supported. */
SOURCE_ABI_EXPORT SourceAbiStatus source_host_run_app_system_group(
	SourceAbiAppSystemFn callback, void *user_data, int32_t *out_result);

/* Split lifecycle: no Main callback. On startup success result=0 and handle!=0;
 * on subsystem failure result=-1, handle=0, and rollback already completed.
 * Retains identity addresses only, not dereferenceable native pointers. The
 * caller must keep native systems alive and shut down on the startup thread
 * with the same callback/user_data pair before unloading the ABI library.
 * Duplicate identities and stale/mismatched/reentrant shutdowns are rejected. */
SOURCE_ABI_EXPORT SourceAbiStatus source_host_app_group_startup(
	SourceAbiAppSystemFn callback, void *user_data, uint64_t *out_handle, int32_t *out_result);
SOURCE_ABI_EXPORT SourceAbiStatus source_host_app_group_shutdown(
	uint64_t handle, SourceAbiAppSystemFn callback, void *user_data);

typedef struct SourceAbiContextConfig {
	uint32_t struct_size;
	uint32_t abi_version;
	SourceAbiLogFn log;
	void *user_data;
} SourceAbiContextConfig;

typedef struct SourceAbiFramePlan {
	uint64_t interpolation_numerator_ns;
	uint64_t interpolation_denominator_ns;
	uint64_t dropped_ns;
	uint32_t tick_count;
	uint32_t reserved;
} SourceAbiFramePlan;

typedef struct SourceAbiFrameLoopInfo {
	uint64_t iteration_count;
	uint32_t exit_reason;
	uint32_t reserved;
} SourceAbiFrameLoopInfo;

typedef struct SourceAbiFramePace {
	uint64_t elapsed_ns;
	uint64_t wait_ns;
	uint32_t ready;
	uint32_t reserved;
} SourceAbiFramePace;

typedef struct SourceAbiHostOperationInfo {
	uint64_t target_length;
	uint64_t landmark_length;
	uint32_t kind;
	uint32_t flags;
} SourceAbiHostOperationInfo;

typedef struct SourceAbiTickPlan {
	double previous_remainder;
	double remainder;
	double next_tick;
	uint32_t tick_count;
	uint32_t reserved;
} SourceAbiTickPlan;

typedef struct SourceAbiWorldInfo {
	uint64_t plane_count;
	uint64_t node_count;
	uint64_t leaf_count;
	uint64_t cluster_count;
} SourceAbiWorldInfo;

typedef struct SourceAbiWorldLeaf {
	uint64_t leaf_index;
	int32_t contents;
	int32_t cluster;
	uint32_t area;
	uint32_t flags;
} SourceAbiWorldLeaf;

typedef struct SourceAbiCvarInfo {
	uint64_t generation;
	uint32_t flags;
	uint32_t reserved;
} SourceAbiCvarInfo;

typedef struct SourceAbiDemoInfo {
	uint64_t command_count;
	uint64_t packet_count;
	int32_t playback_ticks;
	int32_t network_protocol;
	/* String-table sections and entries decoded from the recording; validation
	   fails rather than reporting a demo whose tables do not read back. */
	uint64_t string_table_section_count;
	uint64_t string_table_entry_count;
} SourceAbiDemoInfo;

typedef struct SourceAbiSaveInfo {
	uint64_t embedded_file_count;
	uint64_t embedded_map_state_count;
	uint64_t token_count;
} SourceAbiSaveInfo;

typedef struct SourceAbiNetPacketDecision {
	int32_t incoming_sequence;
	int32_t outgoing_ack;
	int32_t dropped;
	uint32_t accepted;
	uint32_t reason;
	uint32_t reserved;
} SourceAbiNetPacketDecision;

typedef struct SourceAbiNetSequenceAdvance {
	int32_t previous;
	int32_t current;
} SourceAbiNetSequenceAdvance;

/* The packed SPLITPACKET header, and the largest message Rust will rebuild
 * from its pieces. A caller offering less than this may be told its buffer is
 * too small. */
#define SOURCE_ABI_SPLIT_PACKET_HEADER_BYTES 12u
#define SOURCE_ABI_MAX_REASSEMBLED_BYTES 288016u

typedef struct SourceAbiSplitPacketHeader {
	int32_t sequence;
	uint32_t packet_number;
	uint32_t packet_count;
	uint32_t split_size;
} SourceAbiSplitPacketHeader;

typedef struct SourceAbiNetPacketHeader {
	int32_t sequence;
	int32_t outgoing_ack;
	uint32_t flags;
	uint32_t reliable_state;
	uint32_t choked;
	uint32_t challenge;
	uint32_t header_bytes;
	uint32_t accepted;
	uint32_t reason;
	uint32_t reserved;
} SourceAbiNetPacketHeader;

typedef struct SourceAbiNetEncodedPacketHeader {
	uint8_t bytes[20];
	uint32_t length;
	uint32_t flags_offset;
	uint32_t checksum_offset;
	uint32_t checksum_start;
	uint32_t base_flags;
	uint32_t reserved;
} SourceAbiNetEncodedPacketHeader;

typedef struct SourceAbiStringTableUpsert {
	uint32_t index;
	uint32_t entry_count;
	uint32_t created;
	uint32_t user_data_changed;
	int32_t tick_changed;
	uint32_t reserved;
} SourceAbiStringTableUpsert;

typedef struct SourceAbiStringTableChange {
	int32_t tick_changed;
	uint32_t changed;
} SourceAbiStringTableChange;

typedef struct SourceAbiDataTableProperty {
	SourceAbiSlice name;
	SourceAbiSlice reference_name;
	uint32_t property_type;
	uint32_t flags;
	int32_t bit_count;
	uint32_t elements;
	float low_value;
	float high_value;
} SourceAbiDataTableProperty;

typedef struct SourceAbiDataTableRegistration {
	uint32_t table_id;
	uint32_t created;
} SourceAbiDataTableRegistration;

typedef struct SourceAbiDataTableSummary {
	uint32_t table_count;
	uint32_t property_count;
	uint32_t class_count;
	uint32_t compatibility_crc;
} SourceAbiDataTableSummary;

typedef struct SourceAbiSnapshotEntity {
	uint32_t entity_index;
	int32_t serial_number;
	uint32_t class_id;
	uint32_t reserved;
} SourceAbiSnapshotEntity;

typedef struct SourceAbiSnapshotSummary {
	int32_t tick;
	uint32_t max_entities;
	uint32_t valid_entity_count;
	uint32_t explicit_delete_count;
} SourceAbiSnapshotSummary;

enum {
	SOURCE_SNAPSHOT_DELTA_ENTER_PVS = 0,
	SOURCE_SNAPSHOT_DELTA_LEAVE_PVS = 1,
	SOURCE_SNAPSHOT_DELTA_CANDIDATE = 2
};

/* Widest packet-entity header the encoder can produce, in bytes. */
#define SOURCE_MAX_DELTA_HEADER_BYTES 5

typedef struct SourceAbiSnapshotDelta {
	uint32_t entity_index;
	uint32_t kind;
	uint32_t class_id;
	int32_t serial_number;
	uint32_t recreated;
	uint32_t reserved;
} SourceAbiSnapshotDelta;

enum {
	SOURCE_CVAR_READ_ONLY = 1u << 0
};

enum {
	SOURCE_HOST_CREATED = 0,
	SOURCE_HOST_LAUNCHER_READY = 1,
	SOURCE_HOST_CONTENT_READY = 2,
	SOURCE_HOST_LEGACY_RUNNING = 3,
	SOURCE_HOST_SHUTTING_DOWN = 4,
	SOURCE_HOST_STOPPED = 5
};

enum {
	SOURCE_HOST_SESSION_STOP = 0,
	SOURCE_HOST_SESSION_RESTART = 1,
	SOURCE_HOST_SESSION_FAILED = -1
};

enum {
	SOURCE_HOST_FRAME_CONTINUE = 0,
	SOURCE_HOST_FRAME_STOP = 1,
	SOURCE_HOST_FRAME_RESTART = 2,
	SOURCE_HOST_FRAME_FAILED = 3
};

enum {
	SOURCE_HOST_OPERATION_NEW_GAME = 1,
	SOURCE_HOST_OPERATION_LOAD_GAME = 2,
	SOURCE_HOST_OPERATION_CHANGE_LEVEL_SP = 3,
	SOURCE_HOST_OPERATION_CHANGE_LEVEL_MP = 4,
	SOURCE_HOST_OPERATION_GAME_SHUTDOWN = 5,
	SOURCE_HOST_OPERATION_SHUTDOWN = 6,
	SOURCE_HOST_OPERATION_RESTART = 7
};

enum {
	SOURCE_HOST_OPERATION_REMEMBER_LOCATION = 1u << 0,
	SOURCE_HOST_OPERATION_BACKGROUND_LEVEL = 1u << 1
};

enum {
	SOURCE_WORLD_PVS = 0,
	SOURCE_WORLD_PAS = 1
};

enum {
	SOURCE_NET_PACKET_ACCEPTED = 0,
	SOURCE_NET_PACKET_DUPLICATE = 1,
	SOURCE_NET_PACKET_OUT_OF_ORDER = 2,
	SOURCE_NET_PACKET_EXCESSIVE_DROP = 3
};

enum {
	SOURCE_NET_HEADER_ACCEPTED = 0,
	SOURCE_NET_HEADER_TRUNCATED = 1,
	SOURCE_NET_HEADER_CHECKSUM_MISMATCH = 2,
	SOURCE_NET_HEADER_CHALLENGE_MISMATCH = 3,
	SOURCE_NET_HEADER_CHALLENGE_MISSING = 4
};

SOURCE_ABI_EXPORT uint32_t source_abi_version(void);

/* The caller owns the returned handle and must destroy it exactly once. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_create(
	const SourceAbiContextConfig *config,
	SourceAbiHandle *out_handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_destroy(SourceAbiHandle handle);

/* Copies input into caller-owned output. No allocation crosses the ABI. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_echo(
	SourceAbiHandle handle,
	SourceAbiSlice input,
	SourceAbiMutSlice output,
	uint64_t *out_written);

/* The message is borrowed only for the duration of this call. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_emit_log(
	SourceAbiHandle handle,
	int32_t level,
	SourceAbiSlice message);

/* Resolves a validated virtual path below a content root; output is not NUL-terminated. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_resolve_content_path(
	SourceAbiHandle handle,
	SourceAbiSlice root,
	SourceAbiSlice virtual_path,
	SourceAbiMutSlice output,
	uint64_t *out_written);

/* Returns the non-NUL-terminated UTF-8 parent of the current executable. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_executable_base(
	SourceAbiHandle handle,
	SourceAbiMutSlice output,
	uint64_t *out_written);

/* Atomically replaces startup GAME read mounts from the selected gameinfo.txt.
 * external_root is optional (an empty slice); no writable paths are created. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_mount_gameinfo(
	SourceAbiHandle handle,
	SourceAbiSlice base,
	SourceAbiSlice game,
	SourceAbiSlice external_root,
	uint64_t *out_mount_count);

/* Adds a loose directory or VPK to the ordered search paths (at_head is 0 or 1). */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_mount_directory(
	SourceAbiHandle handle,
	SourceAbiSlice root,
	SourceAbiSlice path_id,
	uint8_t at_head);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_mount_vpk(
	SourceAbiHandle handle,
	SourceAbiSlice directory_path,
	SourceAbiSlice path_id,
	uint8_t at_head);

/* Rebuilds the active native read search order; flags are zero or one. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_read_paths_clear(
	SourceAbiHandle handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_read_path_add_directory_flags(
	SourceAbiHandle handle,
	SourceAbiSlice root,
	SourceAbiSlice path_id,
	uint8_t at_head,
	uint8_t by_request_only,
	uint8_t allow_symlink_escape);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_read_path_add_vpk_flags(
	SourceAbiHandle handle,
	SourceAbiSlice directory_path,
	SourceAbiSlice path_id,
	uint8_t at_head,
	uint8_t by_request_only);

/* ZIP byte range in a standalone archive or BSP; Rust owns IO and parsing.
 * Untyped ranges do not qualify for the reserved BSP path-ID selector. Use a
 * typed BSP index and read_path_add_pak_index to mount a discoverable map pack. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_read_path_add_pak_flags(
	SourceAbiHandle handle,
	SourceAbiSlice archive_path,
	uint64_t offset,
	uint64_t length,
	SourceAbiSlice path_id,
	uint8_t at_head,
	uint8_t by_request_only);

/* Synchronizes and resolves the ordered loose-directory write paths. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_write_paths_clear(
	SourceAbiHandle handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_write_path_add(
	SourceAbiHandle handle,
	SourceAbiSlice root,
	SourceAbiSlice path_id,
	uint8_t at_head);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_write_path_add_flags(
	SourceAbiHandle handle,
	SourceAbiSlice root,
	SourceAbiSlice path_id,
	uint8_t at_head,
	uint8_t by_request_only);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_resolve_write_path(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	SourceAbiMutSlice output,
	uint64_t *out_written);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_create_write_directory(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_remove_write_file(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_rename_write_file(
	SourceAbiHandle handle,
	SourceAbiSlice old_virtual_path,
	SourceAbiSlice old_path_id,
	SourceAbiSlice new_virtual_path,
	SourceAbiSlice new_path_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_is_write_file_writable(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	uint32_t *out_writable);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_set_write_file_writable(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	uint8_t writable);

/* Context-owned relative read/write file handles. Seek origins are 0/1/2. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_open_read(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	uint64_t *out_file,
	uint64_t *out_size);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_open_write(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	SourceAbiSlice mode,
	uint64_t *out_file,
	uint64_t *out_size);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_close(
	SourceAbiHandle handle,
	uint64_t file);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_read(
	SourceAbiHandle handle,
	uint64_t file,
	SourceAbiMutSlice output,
	uint64_t *out_read);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_write(
	SourceAbiHandle handle,
	uint64_t file,
	SourceAbiSlice input,
	uint64_t *out_written);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_flush(
	SourceAbiHandle handle,
	uint64_t file);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_seek(
	SourceAbiHandle handle,
	uint64_t file,
	int64_t offset,
	uint32_t origin,
	uint64_t *out_position);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_tell(
	SourceAbiHandle handle,
	uint64_t file,
	uint64_t *out_position);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_open_file_size(
	SourceAbiHandle handle,
	uint64_t file,
	uint64_t *out_size);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_is_open(
	SourceAbiHandle handle,
	uint64_t file,
	uint32_t *out_open);

/* Reads through mounted search paths; an empty path ID selects every mount. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_read_file(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	SourceAbiMutSlice output,
	uint64_t *out_written);

/* Returns a mounted file's size without reading its payload. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_file_size(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	uint64_t *out_size);

/* Resolves a relative path to a canonical loose or encoded VPK path. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_resolve_read_path(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	SourceAbiMutSlice output,
	uint64_t *out_written,
	uint32_t *out_kind);

/* Reports whether a mounted relative path names a loose or VPK directory. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_path_is_directory(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	uint32_t *out_is_directory);

/* Context-owned wildcard cursors over mounted loose and VPK paths. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_find_first(
	SourceAbiHandle handle,
	SourceAbiSlice wildcard,
	SourceAbiSlice path_id,
	SourceAbiMutSlice output,
	uint64_t *out_written,
	uint32_t *out_is_directory,
	uint64_t *out_find);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_find_next(
	SourceAbiHandle handle,
	uint64_t find,
	SourceAbiMutSlice output,
	uint64_t *out_written,
	uint32_t *out_is_directory);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_find_close(
	SourceAbiHandle handle,
	uint64_t find);

/* Context-owned sequence/drop state for transitional native network channels. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_channel_create(
	SourceAbiHandle handle,
	int32_t outgoing_sequence,
	int32_t incoming_sequence,
	int32_t outgoing_ack,
	uint64_t *out_channel_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_channel_remove(
	SourceAbiHandle handle,
	uint64_t channel_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_channel_reset(
	SourceAbiHandle handle,
	uint64_t channel_id,
	int32_t outgoing_sequence,
	int32_t incoming_sequence,
	int32_t outgoing_ack);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_channel_preview_incoming(
	SourceAbiHandle handle,
	uint64_t channel_id,
	int32_t sequence,
	int32_t outgoing_ack,
	uint32_t choked,
	int32_t max_drop,
	SourceAbiNetPacketDecision *out_decision);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_channel_commit_incoming(
	SourceAbiHandle handle,
	uint64_t channel_id,
	int32_t sequence,
	int32_t outgoing_ack,
	uint32_t choked,
	int32_t max_drop,
	SourceAbiNetPacketDecision *out_decision);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_channel_advance_outgoing(
	SourceAbiHandle handle,
	uint64_t channel_id,
	SourceAbiNetSequenceAdvance *out_advance);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_packet_checksum(
	SourceAbiHandle handle,
	SourceAbiSlice payload,
	uint16_t *out_checksum);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_packet_encode_header(
	SourceAbiHandle handle,
	int32_t sequence,
	int32_t outgoing_ack,
	uint32_t reliable_state,
	uint32_t has_choked,
	uint32_t choked,
	uint32_t has_challenge,
	uint32_t challenge,
	uint32_t checksum_required,
	SourceAbiNetEncodedPacketHeader *out_header);
/* DEFAULT_LZSS_WINDOW_SIZE, which is what the engine's generic buffer
 * compression uses; the save system passes a smaller one. */
#define SOURCE_ABI_LZSS_DEFAULT_WINDOW 4096u

SOURCE_ABI_EXPORT SourceAbiStatus source_compress_lzss_compress(
	SourceAbiSlice input,
	uint32_t window,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint64_t *out_length);
SOURCE_ABI_EXPORT SourceAbiStatus source_compress_lzss_decompress(
	SourceAbiSlice input,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint64_t *out_length);
SOURCE_ABI_EXPORT SourceAbiStatus source_compress_lzss_actual_size(
	SourceAbiSlice input,
	uint64_t *out_actual_size);
/* Snappy uses Source's SNAP tag, not the Snappy framing format. Compression
 * may grow data. All buffers are caller-owned and input/output cannot overlap.
 * Zero-capacity null output queries report BUFFER_TOO_SMALL and required size.
 * Generic decode accepts SNAP, LZSS, and untagged bytes; tagged corruption is
 * FORMAT_ERROR and is never copied as plain data. Failed decode leaves output
 * bytes untouched. actual_size checks only headers and returns DECLINED for
 * untagged input. Length outputs must always be nonnull. */
SOURCE_ABI_EXPORT SourceAbiStatus source_compress_snappy_max_size(
	uint64_t input_length, uint64_t *out_size);
SOURCE_ABI_EXPORT SourceAbiStatus source_compress_snappy_compress(
	SourceAbiSlice input, uint8_t *out_bytes, uint64_t capacity, uint64_t *out_length);
SOURCE_ABI_EXPORT SourceAbiStatus source_compress_buffer_actual_size(
	SourceAbiSlice input, uint64_t *out_size);
SOURCE_ABI_EXPORT SourceAbiStatus source_compress_buffer_decompress(
	SourceAbiSlice input, uint8_t *out_bytes, uint64_t capacity, uint64_t *out_length);
SOURCE_ABI_EXPORT SourceAbiStatus source_net_split_packet_header_encode(
	int32_t sequence,
	uint32_t packet_number,
	uint32_t packet_count,
	uint32_t split_size,
	uint8_t *out_bytes);
SOURCE_ABI_EXPORT SourceAbiStatus source_net_split_packet_header_decode(
	SourceAbiSlice datagram,
	SourceAbiSplitPacketHeader *out_header);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_split_packet_create(
	SourceAbiHandle handle,
	uint64_t *out_peer_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_split_packet_remove(
	SourceAbiHandle handle,
	uint64_t peer_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_split_packet_reset(
	SourceAbiHandle handle,
	uint64_t peer_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_split_packet_accept(
	SourceAbiHandle handle,
	uint64_t peer_id,
	SourceAbiSlice datagram,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint64_t *out_length,
	uint32_t *out_complete);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_split_packet_collect(
	SourceAbiHandle handle,
	uint64_t peer_id,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint64_t *out_length);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_packet_finalize_header(
	SourceAbiHandle handle,
	SourceAbiMutSlice packet,
	uint32_t flags,
	uint32_t checksum_required,
	uint16_t *out_checksum);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_net_packet_header(
	SourceAbiHandle handle,
	SourceAbiSlice packet,
	uint32_t checksum_required,
	uint32_t expects_challenge,
	uint32_t expected_challenge,
	SourceAbiNetPacketHeader *out_header);

/* Canonical network string-table contents, indexes, user data, and change ticks. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_create(
	SourceAbiHandle handle,
	SourceAbiSlice name,
	uint32_t max_entries,
	int32_t tick,
	uint64_t *out_table_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_remove(
	SourceAbiHandle handle,
	uint64_t table_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_clear(
	SourceAbiHandle handle,
	uint64_t table_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_enable_history(
	SourceAbiHandle handle,
	uint64_t table_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_set_tick(
	SourceAbiHandle handle,
	uint64_t table_id,
	int32_t tick);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_synchronize_tick(
	SourceAbiHandle handle,
	uint64_t table_id,
	int32_t tick);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_upsert(
	SourceAbiHandle handle,
	uint64_t table_id,
	SourceAbiSlice value,
	uint32_t update_user_data,
	SourceAbiSlice user_data,
	SourceAbiStringTableUpsert *out_result);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_set_user_data(
	SourceAbiHandle handle,
	uint64_t table_id,
	uint32_t index,
	SourceAbiSlice user_data,
	SourceAbiStringTableChange *out_change);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_find(
	SourceAbiHandle handle,
	uint64_t table_id,
	SourceAbiSlice value,
	uint32_t *out_index);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_changed_since(
	SourceAbiHandle handle,
	uint64_t table_id,
	int32_t tick,
	uint32_t *out_changed);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_restore_tick(
	SourceAbiHandle handle,
	uint64_t table_id,
	int32_t tick,
	int32_t *out_last_changed_tick);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_entry_count(
	SourceAbiHandle handle,
	uint64_t table_id,
	uint32_t *out_entry_count);
/* Encodes a table's canonical entries in the layout demo and save containers
   use: a 16-bit count, then per entry a NUL-terminated name, a flag bit, and,
   when set, a 16-bit length followed by that many user data bytes. The bits are
   packed least-significant-bit first within each byte, so they can be appended
   unchanged and followed by the caller's own client-side section. The required
   bit count is reported even when the buffer is too small. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_encode_entries(
	SourceAbiHandle handle,
	uint64_t table_id,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint32_t *out_bit_count);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_read_string(
	SourceAbiHandle handle,
	uint64_t table_id,
	uint32_t index,
	SourceAbiMutSlice output,
	uint64_t *out_written);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_string_table_read_user_data(
	SourceAbiHandle handle,
	uint64_t table_id,
	uint32_t index,
	SourceAbiMutSlice output,
	uint64_t *out_written);

/* Canonical server datatable metadata and contiguous server-class IDs. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_data_table_clear(
	SourceAbiHandle handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_data_table_register(
	SourceAbiHandle handle,
	SourceAbiSlice name,
	uint32_t property_count,
	SourceAbiDataTableRegistration *out_registration);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_data_table_register_property(
	SourceAbiHandle handle,
	uint32_t table_id,
	const SourceAbiDataTableProperty *property);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_server_class_register(
	SourceAbiHandle handle,
	SourceAbiSlice name,
	uint32_t table_id,
	uint32_t *out_class_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_data_table_finalize(
	SourceAbiHandle handle,
	uint32_t native_compatibility_crc,
	SourceAbiDataTableSummary *out_summary);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_data_table_summary(
	SourceAbiHandle handle,
	SourceAbiDataTableSummary *out_summary);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_server_class_find(
	SourceAbiHandle handle,
	SourceAbiSlice name,
	uint32_t *out_class_id);

/* Immutable canonical server-frame snapshot metadata and entity ordering. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_snapshot_clear(
	SourceAbiHandle handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_snapshot_queue_delete(
	SourceAbiHandle handle,
	uint32_t slot,
	uint32_t *out_queued);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_snapshot_create(
	SourceAbiHandle handle,
	int32_t tick,
	uint32_t max_entities,
	const SourceAbiSnapshotEntity *entities,
	uint64_t entity_count,
	uint64_t *out_snapshot_id,
	SourceAbiSnapshotSummary *out_summary);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_snapshot_remove(
	SourceAbiHandle handle,
	uint64_t snapshot_id);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_snapshot_summary(
	SourceAbiHandle handle,
	uint64_t snapshot_id,
	SourceAbiSnapshotSummary *out_summary);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_snapshot_entity_at(
	SourceAbiHandle handle,
	uint64_t snapshot_id,
	uint32_t ordinal,
	SourceAbiSnapshotEntity *out_entity);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_snapshot_delete_at(
	SourceAbiHandle handle,
	uint64_t snapshot_id,
	uint32_t ordinal,
	uint32_t *out_slot);
/* Classifies the entity indices that differ between two snapshots. A
   from_snapshot_id of zero requests a full update. Each side may carry an
   ascending, duplicate-free visibility set, or a null pointer to compare the
   snapshot's whole entity list. The required entry count is always reported, so
   a short buffer returns SOURCE_ABI_BUFFER_TOO_SMALL along with the size the
   caller needs. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_snapshot_delta(
	SourceAbiHandle handle,
	uint64_t from_snapshot_id,
	const uint32_t *from_visible,
	uint64_t from_visible_count,
	uint64_t to_snapshot_id,
	const uint32_t *to_visible,
	uint64_t to_visible_count,
	SourceAbiSnapshotDelta *out_deltas,
	uint64_t capacity,
	uint64_t *out_count);
/* Encodes a demo file header into the exact bytes a demo starts with. Each
   name is a UTF-8 slice that has to fit its fixed field with room for a
   terminator. The encoder refuses a header the Rust parser would reject, so a
   recorder cannot emit a demo it could not read back. */
SOURCE_ABI_EXPORT SourceAbiStatus source_demo_header_encode(
	int32_t demo_protocol,
	int32_t network_protocol,
	SourceAbiSlice server_name,
	SourceAbiSlice client_name,
	SourceAbiSlice map_name,
	SourceAbiSlice game_directory,
	float playback_time,
	int32_t playback_ticks,
	int32_t playback_frames,
	int32_t signon_length,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint64_t *out_size);
/* Encodes one packet-entity header into protocol-25 wire bits. header_base is
   the index of the previously written entity, or -1 before the first one. The
   bits are packed least-significant-bit first within each byte and low byte
   first, so they can be appended to a packet unchanged. */
SOURCE_ABI_EXPORT SourceAbiStatus source_delta_header_encode(
	uint32_t entity_index,
	int32_t header_base,
	uint32_t leave_pvs,
	uint32_t delete_entity,
	uint32_t enter_pvs,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint32_t *out_bit_count);

/* Reads and parses a mounted scene-image cache. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_probe_scene(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	uint64_t *out_scene_count,
	uint64_t *out_string_count);

/* Reads and structurally validates mounted demo/save artifacts. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_demo_validate(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	SourceAbiDemoInfo *out_info);
/* Composes a map-state save container from its token table and two data
   sections. token_count must match the NUL-terminated names in tokens. The
   required size is reported even when the buffer is too small, and input the
   reader would reject is refused. */
SOURCE_ABI_EXPORT SourceAbiStatus source_save_map_state_encode(
	SourceAbiSlice tokens,
	uint64_t token_count,
	SourceAbiSlice data_headers,
	SourceAbiSlice data,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint64_t *out_size);
/* Composes the leading save container: magic, version, sizes, token table and
   game data. The embedded map states are appended by the caller afterwards. */
SOURCE_ABI_EXPORT SourceAbiStatus source_save_container_header_encode(
	SourceAbiSlice tokens,
	uint64_t token_count,
	SourceAbiSlice game_data,
	uint8_t *out_bytes,
	uint64_t capacity,
	uint64_t *out_size);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_save_validate(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	SourceAbiSaveInfo *out_info);

/* Loads and queries the owned plane/node/leaf/visibility subset of a BSP. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_world_load(
	SourceAbiHandle handle,
	SourceAbiSlice virtual_path,
	SourceAbiSlice path_id,
	SourceAbiWorldInfo *out_info);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_world_clear(SourceAbiHandle handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_world_point_leaf(
	SourceAbiHandle handle,
	float x,
	float y,
	float z,
	SourceAbiWorldLeaf *out_leaf);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_world_visibility(
	SourceAbiHandle handle,
	uint32_t cluster,
	uint32_t kind,
	SourceAbiMutSlice output,
	uint64_t *out_written);

/* Validated host lifecycle plus deterministic fixed-step scheduling. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_transition(
	SourceAbiHandle handle,
	uint32_t phase);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_phase(
	SourceAbiHandle handle,
	uint32_t *out_phase);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_configure_scheduler(
	SourceAbiHandle handle,
	uint64_t tick_interval_ns,
	uint32_t max_catch_up_ticks);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_advance(
	SourceAbiHandle handle,
	uint64_t now_ns,
	SourceAbiFramePlan *out_plan);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_pace(
	SourceAbiHandle handle,
	uint64_t now_ns,
	uint64_t minimum_frame_ns,
	SourceAbiFramePace *out_pace);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_schedule_ticks(
	SourceAbiHandle handle,
	double frame_time,
	double tick_interval,
	int32_t start_tick,
	uint32_t accumulate,
	uint32_t alternate_ticks,
	SourceAbiTickPlan *out_plan);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_request_operation(
	SourceAbiHandle handle,
	uint32_t kind,
	SourceAbiSlice target,
	SourceAbiSlice landmark,
	uint32_t flags);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_take_operation(
	SourceAbiHandle handle,
	SourceAbiMutSlice target_output,
	SourceAbiMutSlice landmark_output,
	SourceAbiHostOperationInfo *out_info);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_pending_operation(
	SourceAbiHandle handle,
	uint32_t *out_kind);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_clear_operation(
	SourceAbiHandle handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_run_sessions(
	SourceAbiHandle handle,
	SourceAbiSessionFn run_session,
	void *user_data,
	uint32_t max_sessions,
	uint32_t *out_session_count);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_host_run_frames(
	SourceAbiHandle handle,
	SourceAbiFrameFn run_frame,
	void *user_data,
	uint64_t max_iterations,
	SourceAbiFrameLoopInfo *out_info);

/* Context-owned command/cvar state with bounded UTF-8 inputs and caller-owned output. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_cvar_register(
	SourceAbiHandle handle,
	SourceAbiSlice name,
	SourceAbiSlice default_value,
	uint32_t flags);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_cvar_set(
	SourceAbiHandle handle,
	SourceAbiSlice name,
	SourceAbiSlice value);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_cvar_get(
	SourceAbiHandle handle,
	SourceAbiSlice name,
	SourceAbiMutSlice output,
	uint64_t *out_written,
	SourceAbiCvarInfo *out_info);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_command_enqueue(
	SourceAbiHandle handle,
	SourceAbiSlice script,
	uint32_t *out_command_count);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_command_pop(
	SourceAbiHandle handle,
	SourceAbiMutSlice output,
	uint64_t *out_written);

/* Stateful input normalization used by the transitional SDL adapter. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_input_modifier(
	SourceAbiHandle handle,
	uint32_t modifier,
	uint8_t pressed,
	uint32_t *out_modifier_mask);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_input_mouse_button(
	SourceAbiHandle handle,
	uint32_t button,
	uint8_t pressed,
	uint32_t *out_button_mask);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_input_mouse_motion(
	SourceAbiHandle handle,
	int32_t delta_x,
	int32_t delta_y);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_input_take_mouse_delta(
	SourceAbiHandle handle,
	int32_t *out_delta_x,
	int32_t *out_delta_y);
SOURCE_ABI_EXPORT SourceAbiStatus source_context_input_reset(SourceAbiHandle handle);

/* Parses a VPK directory file from disk and returns its directory metadata. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_probe_vpk(
	SourceAbiHandle handle,
	SourceAbiSlice path,
	uint64_t *out_entry_count,
	uint32_t *out_version);

/* Reads and CRC-validates one VPK member into caller-owned storage. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_read_vpk_file(
	SourceAbiHandle handle,
	SourceAbiSlice directory_path,
	SourceAbiSlice entry_path,
	SourceAbiMutSlice output,
	uint64_t *out_written);

/* Reads, CRC-validates, and parses a scene-image cache stored in a VPK. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_probe_vpk_scene(
	SourceAbiHandle handle,
	SourceAbiSlice directory_path,
	SourceAbiSlice entry_path,
	uint64_t *out_scene_count,
	uint64_t *out_string_count);

SOURCE_ABI_EXPORT uint64_t source_context_live_count(void);

/* ABI-lab hook: deliberately panics internally and must return SOURCE_ABI_PANIC. */
SOURCE_ABI_EXPORT SourceAbiStatus source_context_force_panic_for_test(SourceAbiHandle handle);

/* Presenting Metal frames into the window the engine owns.
 *
 * The renderer could already draw into a layer of its own and read it back
 * offscreen. These hand it the window's own view instead, which is what
 * lets it draw what a player sees. A presenter belongs to the thread that
 * created it, because AppKit requires view changes on the main thread;
 * used from any other it reports SOURCE_ABI_INVALID_HANDLE rather than
 * appearing to work. */

/* `window` must be a live NSWindow that outlives the presenter, and this
 * must be called on the main thread. The window is taken rather than its
 * view so that the caller never has to send an Objective-C message: every
 * one of those stays on the Rust side. The caller owns the returned handle
 * and must destroy it exactly once, before the window goes away. */
SOURCE_ABI_EXPORT SourceAbiStatus source_render_presenter_create(
	void *window,
	uint32_t width,
	uint32_t height,
	double scale,
	SourceAbiHandle *out_handle);
SOURCE_ABI_EXPORT SourceAbiStatus source_render_presenter_destroy(SourceAbiHandle handle);

/* Size and backing scale together, because a window moved between a Retina
 * display and an external one changes scale without changing its size in
 * points, and a layer tracking only one draws at the wrong resolution. */
SOURCE_ABI_EXPORT SourceAbiStatus source_render_presenter_resize(
	SourceAbiHandle handle,
	uint32_t width,
	uint32_t height,
	double scale);

SOURCE_ABI_EXPORT SourceAbiStatus source_render_presenter_present(
	SourceAbiHandle handle,
	float red,
	float green,
	float blue);

/* What a presented frame drew, so a caller can tell a frame with a world in
 * it from a bare clear without reading the pixels back. */
typedef struct SourceAbiWorldDraw
{
	uint64_t batches;
	uint64_t triangles;
	uint64_t map_triangles;
	uint64_t materials;
	uint64_t prop_triangles;
	uint64_t props;
} SourceAbiWorldDraw;

/* Loads a map onto the presenter's device, read through the context's own
 * content mounts so it resolves as it does for the rest of the engine. */
SOURCE_ABI_EXPORT SourceAbiStatus source_render_world_load(
	SourceAbiHandle handle,
	SourceAbiHandle context,
	SourceAbiSlice map,
	SourceAbiWorldDraw *out_drawn);

/* Draws the loaded map from the engine's own view and presents it.
 * `position` and `angles` are each three floats, in the engine's units and
 * its pitch-yaw-roll order. */
SOURCE_ABI_EXPORT SourceAbiStatus source_render_ui_begin( SourceAbiHandle handle );

SOURCE_ABI_EXPORT SourceAbiStatus source_render_ui_texture(
	SourceAbiHandle handle,
	uint32_t id,
	uint32_t width,
	uint32_t height,
	const uint8_t *rgba );

SOURCE_ABI_EXPORT SourceAbiStatus source_render_ui_texture_region(
	SourceAbiHandle handle,
	uint32_t id,
	uint32_t x,
	uint32_t y,
	uint32_t width,
	uint32_t height,
	const uint8_t *rgba );

SOURCE_ABI_EXPORT SourceAbiStatus source_render_ui_texture_alias(
	SourceAbiHandle handle,
	uint32_t alias,
	uint32_t base );

SOURCE_ABI_EXPORT int32_t source_render_ui_has_texture( SourceAbiHandle handle, uint32_t id );

SOURCE_ABI_EXPORT SourceAbiStatus source_render_ui_quad(
	SourceAbiHandle handle,
	uint32_t texture,
	const float *bounds,
	const float *coords,
	const float *tint );

SOURCE_ABI_EXPORT SourceAbiStatus source_render_ui_end(
	SourceAbiHandle handle,
	uint32_t width,
	uint32_t height,
	uint64_t *out_quads );

SOURCE_ABI_EXPORT SourceAbiStatus source_render_world_present(
	SourceAbiHandle handle,
	const float *position,
	const float *angles,
	SourceAbiWorldDraw *out_drawn);

SOURCE_ABI_EXPORT SourceAbiStatus source_render_presenter_drawable_size(
	SourceAbiHandle handle,
	uint32_t *out_width,
	uint32_t *out_height);

#ifdef __cplusplus
} /* extern "C" */

static_assert(sizeof(SourceAbiHandle) == 8, "SourceAbiHandle layout changed");
static_assert(sizeof(SourceAbiSlice) == 16, "SourceAbiSlice layout changed");
static_assert(sizeof(SourceAbiFramePlan) == 32, "SourceAbiFramePlan layout changed");
static_assert(sizeof(SourceAbiFrameLoopInfo) == 16, "SourceAbiFrameLoopInfo layout changed");
static_assert(sizeof(SourceAbiFramePace) == 24, "SourceAbiFramePace layout changed");
static_assert(sizeof(SourceAbiHostOperationInfo) == 24, "SourceAbiHostOperationInfo layout changed");
static_assert(sizeof(SourceAbiTickPlan) == 32, "SourceAbiTickPlan layout changed");
static_assert(sizeof(SourceAbiWorldInfo) == 32, "SourceAbiWorldInfo layout changed");
static_assert(sizeof(SourceAbiWorldLeaf) == 24, "SourceAbiWorldLeaf layout changed");
static_assert(sizeof(SourceAbiCvarInfo) == 16, "SourceAbiCvarInfo layout changed");
static_assert(sizeof(SourceAbiDemoInfo) == 40, "SourceAbiDemoInfo layout changed");
static_assert(sizeof(SourceAbiSaveInfo) == 24, "SourceAbiSaveInfo layout changed");
static_assert(sizeof(SourceAbiNetPacketDecision) == 24, "SourceAbiNetPacketDecision layout changed");
static_assert(sizeof(SourceAbiNetSequenceAdvance) == 8, "SourceAbiNetSequenceAdvance layout changed");
static_assert(sizeof(SourceAbiNetPacketHeader) == 40, "SourceAbiNetPacketHeader layout changed");
static_assert(sizeof(SourceAbiNetEncodedPacketHeader) == 44, "SourceAbiNetEncodedPacketHeader layout changed");
static_assert(sizeof(SourceAbiStringTableUpsert) == 24, "SourceAbiStringTableUpsert layout changed");
static_assert(sizeof(SourceAbiStringTableChange) == 8, "SourceAbiStringTableChange layout changed");
static_assert(sizeof(SourceAbiDataTableProperty) == 56, "SourceAbiDataTableProperty layout changed");
static_assert(sizeof(SourceAbiDataTableRegistration) == 8, "SourceAbiDataTableRegistration layout changed");
static_assert(sizeof(SourceAbiDataTableSummary) == 16, "SourceAbiDataTableSummary layout changed");
static_assert(sizeof(SourceAbiSnapshotEntity) == 16, "SourceAbiSnapshotEntity layout changed");
static_assert(sizeof(SourceAbiSnapshotSummary) == 16, "SourceAbiSnapshotSummary layout changed");
static_assert(sizeof(SourceAbiSnapshotDelta) == 24, "SourceAbiSnapshotDelta layout changed");
static_assert(offsetof(SourceAbiContextConfig, log) == 8, "SourceAbiContextConfig layout changed");
#elif defined(__STDC_VERSION__) && __STDC_VERSION__ >= 201112L
_Static_assert(sizeof(SourceAbiHandle) == 8, "SourceAbiHandle layout changed");
_Static_assert(sizeof(SourceAbiSlice) == 16, "SourceAbiSlice layout changed");
_Static_assert(sizeof(SourceAbiFramePlan) == 32, "SourceAbiFramePlan layout changed");
_Static_assert(sizeof(SourceAbiFrameLoopInfo) == 16, "SourceAbiFrameLoopInfo layout changed");
_Static_assert(sizeof(SourceAbiFramePace) == 24, "SourceAbiFramePace layout changed");
_Static_assert(sizeof(SourceAbiHostOperationInfo) == 24, "SourceAbiHostOperationInfo layout changed");
_Static_assert(sizeof(SourceAbiTickPlan) == 32, "SourceAbiTickPlan layout changed");
_Static_assert(sizeof(SourceAbiWorldInfo) == 32, "SourceAbiWorldInfo layout changed");
_Static_assert(sizeof(SourceAbiWorldLeaf) == 24, "SourceAbiWorldLeaf layout changed");
_Static_assert(sizeof(SourceAbiCvarInfo) == 16, "SourceAbiCvarInfo layout changed");
_Static_assert(sizeof(SourceAbiDemoInfo) == 40, "SourceAbiDemoInfo layout changed");
_Static_assert(sizeof(SourceAbiSaveInfo) == 24, "SourceAbiSaveInfo layout changed");
_Static_assert(sizeof(SourceAbiNetPacketDecision) == 24, "SourceAbiNetPacketDecision layout changed");
_Static_assert(sizeof(SourceAbiNetSequenceAdvance) == 8, "SourceAbiNetSequenceAdvance layout changed");
_Static_assert(sizeof(SourceAbiNetPacketHeader) == 40, "SourceAbiNetPacketHeader layout changed");
_Static_assert(sizeof(SourceAbiNetEncodedPacketHeader) == 44, "SourceAbiNetEncodedPacketHeader layout changed");
_Static_assert(sizeof(SourceAbiStringTableUpsert) == 24, "SourceAbiStringTableUpsert layout changed");
_Static_assert(sizeof(SourceAbiStringTableChange) == 8, "SourceAbiStringTableChange layout changed");
_Static_assert(sizeof(SourceAbiDataTableProperty) == 56, "SourceAbiDataTableProperty layout changed");
_Static_assert(sizeof(SourceAbiDataTableRegistration) == 8, "SourceAbiDataTableRegistration layout changed");
_Static_assert(sizeof(SourceAbiDataTableSummary) == 16, "SourceAbiDataTableSummary layout changed");
_Static_assert(sizeof(SourceAbiSnapshotEntity) == 16, "SourceAbiSnapshotEntity layout changed");
_Static_assert(sizeof(SourceAbiSnapshotSummary) == 16, "SourceAbiSnapshotSummary layout changed");
_Static_assert(sizeof(SourceAbiSnapshotDelta) == 24, "SourceAbiSnapshotDelta layout changed");
_Static_assert(offsetof(SourceAbiContextConfig, log) == 8, "SourceAbiContextConfig layout changed");
#endif

#endif
