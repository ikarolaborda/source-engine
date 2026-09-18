#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
TARGET_DIR=${CARGO_TARGET_DIR:-"$ROOT/target"}

cargo build --manifest-path "$ROOT/Cargo.toml" -p source-abi

case "$(uname -s)" in
	Darwin)
		DYLIB="$TARGET_DIR/debug/libsource_abi.dylib"
		RPATH_FLAG="-Wl,-rpath,$TARGET_DIR/debug"
		EXPORTED_SYMBOLS=$(nm -gU "$DYLIB")
		;;
	Linux)
		DYLIB="$TARGET_DIR/debug/libsource_abi.so"
		RPATH_FLAG="-Wl,-rpath,$TARGET_DIR/debug"
		EXPORTED_SYMBOLS=$(nm -D --defined-only "$DYLIB")
		;;
	*)
		echo "verify_abi.sh currently supports macOS and Linux" >&2
		exit 2
		;;
esac

ABI_SYMBOLS="
	source_abi_version \
	source_context_create \
	source_context_destroy \
	source_context_echo \
	source_context_emit_log \
	source_context_resolve_content_path \
	source_context_executable_base \
	source_context_mount_directory \
	source_context_mount_vpk \
	source_context_read_paths_clear \
	source_context_read_path_add_directory_flags \
	source_context_read_path_add_vpk_flags \
	source_context_write_paths_clear \
	source_context_write_path_add \
	source_context_write_path_add_flags \
	source_context_resolve_write_path \
	source_context_create_write_directory \
	source_context_remove_write_file \
	source_context_rename_write_file \
	source_context_is_write_file_writable \
	source_context_set_write_file_writable \
	source_context_file_open_read \
	source_context_file_open_write \
	source_context_file_close \
	source_context_file_read \
	source_context_file_write \
	source_context_file_flush \
	source_context_file_seek \
	source_context_file_tell \
	source_context_open_file_size \
	source_context_file_is_open \
	source_context_file_size \
	source_context_resolve_read_path \
	source_context_path_is_directory \
	source_context_find_first \
	source_context_find_next \
	source_context_find_close \
	source_context_read_file \
	source_context_net_channel_create \
	source_context_net_channel_remove \
	source_context_net_channel_reset \
	source_context_net_channel_preview_incoming \
	source_context_net_channel_commit_incoming \
	source_context_net_channel_advance_outgoing \
	source_context_net_packet_encode_header \
	source_context_net_packet_checksum \
	source_context_net_packet_finalize_header \
	source_compress_lzss_compress \
	source_compress_lzss_decompress \
	source_compress_lzss_actual_size \
	source_net_split_packet_header_encode \
	source_net_split_packet_header_decode \
	source_context_split_packet_create \
	source_context_split_packet_remove \
	source_context_split_packet_reset \
	source_context_split_packet_accept \
	source_context_split_packet_collect \
	source_context_net_packet_header \
	source_context_string_table_create \
	source_context_string_table_remove \
	source_context_string_table_clear \
	source_context_string_table_enable_history \
	source_context_string_table_set_tick \
	source_context_string_table_synchronize_tick \
	source_context_string_table_upsert \
	source_context_string_table_set_user_data \
	source_context_string_table_find \
	source_context_string_table_changed_since \
	source_context_string_table_restore_tick \
	source_context_string_table_entry_count \
	source_context_string_table_read_string \
	source_context_string_table_read_user_data \
	source_context_string_table_encode_entries \
	source_save_map_state_encode \
	source_save_container_header_encode \
	source_context_data_table_clear \
	source_context_data_table_register \
	source_context_data_table_register_property \
	source_context_server_class_register \
	source_context_data_table_finalize \
	source_context_data_table_summary \
	source_context_server_class_find \
	source_context_snapshot_clear \
	source_context_snapshot_queue_delete \
	source_context_snapshot_create \
	source_context_snapshot_remove \
	source_context_snapshot_summary \
	source_context_snapshot_entity_at \
	source_context_snapshot_delete_at \
	source_context_snapshot_delta \
	source_delta_header_encode \
	source_demo_header_encode \
	source_context_probe_scene \
	source_context_demo_validate \
	source_context_save_validate \
	source_context_world_load \
	source_context_world_clear \
	source_context_world_point_leaf \
	source_context_world_visibility \
	source_context_host_transition \
	source_context_host_phase \
	source_context_host_configure_scheduler \
	source_context_host_advance \
	source_context_host_pace \
	source_context_host_schedule_ticks \
	source_context_host_request_operation \
	source_context_host_take_operation \
	source_context_host_pending_operation \
	source_context_host_clear_operation \
	source_context_host_run_sessions \
	source_context_host_run_frames \
	source_context_cvar_register \
	source_context_cvar_set \
	source_context_cvar_get \
	source_context_command_enqueue \
	source_context_command_pop \
	source_context_input_modifier \
	source_context_input_mouse_button \
	source_context_input_mouse_motion \
	source_context_input_take_mouse_delta \
	source_context_input_reset \
	source_context_probe_vpk \
	source_context_read_vpk_file \
	source_context_probe_vpk_scene \
	source_context_live_count \
	source_context_force_panic_for_test
"

for symbol in $ABI_SYMBOLS
do
	echo "$EXPORTED_SYMBOLS" | grep "_$symbol\| $symbol$" >/dev/null || {
		echo "missing exported ABI symbol: $symbol" >&2
		exit 1
	}
done

# The list above only proves the named symbols still exist.  Checking the
# library's own exports back against it is what catches a new entry point that
# was added without a gate, which is how the encoding functions went unchecked.
for exported in $(echo "$EXPORTED_SYMBOLS" | sed -n 's/.*[ 	]_\{0,1\}\(source_[a-z_0-9]*\)$/\1/p' | sort -u)
do
	case " $(echo $ABI_SYMBOLS) " in
		*" $exported "*) ;;
		*)
			echo "exported ABI symbol is not gated: $exported" >&2
			echo "add it to ABI_SYMBOLS in $0" >&2
			exit 1
			;;
	esac
done

CXX=${CXX:-c++}
CC=${CC:-cc}
"$CXX" -std=c++11 -Wall -Wextra -Werror \
	-I"$ROOT" -I"$ROOT/public" \
	"$ROOT/rust/tests/abi_smoke.cpp" \
	"$ROOT/appframework/rust_engine_bridge.cpp" \
	-L"$TARGET_DIR/debug" -lsource_abi "$RPATH_FLAG" \
	-o "$TARGET_DIR/abi_smoke"
if [ "$#" -gt 1 ]; then
	echo "usage: $0 [half-life-2-content-root]" >&2
	exit 2
elif [ "$#" -eq 1 ]; then
	"$TARGET_DIR/abi_smoke" "$1"
else
	"$TARGET_DIR/abi_smoke"
fi

"$CC" -std=c11 -Wall -Wextra -Werror \
	-I"$ROOT/public" "$ROOT/rust/tests/abi_unload_smoke.c" \
	-o "$TARGET_DIR/abi_unload_smoke"
"$TARGET_DIR/abi_unload_smoke" "$DYLIB"

# The LZSS differential links the real tier1 codec next to the Rust one and
# requires them to produce identical bytes. Agreeing on the format is not
# enough for a save file: the two encoders have to make the same match choices
# or the same input yields different files depending on which side wrote it.
# tier1/lzss.cpp is compiled here with the platform defines the engine build
# uses rather than taken from the build tree, so this gate does not require a
# prior Waf build.
NATIVE_DEFINES="-DOSX=1 -D_OSX=1 -DPOSIX=1 -D_POSIX=1 -DPLATFORM_POSIX=1"
case "$(uname -s)" in
	Linux) NATIVE_DEFINES="-DLINUX=1 -D_LINUX=1 -DPOSIX=1 -D_POSIX=1 -DPLATFORM_POSIX=1" ;;
esac
NATIVE_DEFINES="$NATIVE_DEFINES -DGNUC -DNO_HOOK_MALLOC -DPLATFORM_64BITS=1 -DNDEBUG"
NATIVE_DEFINES="$NATIVE_DEFINES -DTIER1_STATIC_LIB=1 -DNO_MEMOVERRIDE_NEW_DELETE=1"
# The legacy translation unit is compiled without the strict warning set,
# because it is the code being compared against rather than code this port
# owns; the harness beside it is still held to it.
# shellcheck disable=SC2086
"$CXX" -std=c++11 $NATIVE_DEFINES \
	-I"$ROOT" -I"$ROOT/public" -I"$ROOT/common" \
	-c "$ROOT/tier1/lzss.cpp" -o "$TARGET_DIR/lzss_native.o"
"$CC" -std=c11 -Wall -Wextra -Werror \
	-c "$ROOT/rust/tests/lzss_native_stub.c" -o "$TARGET_DIR/lzss_native_stub.o"
# shellcheck disable=SC2086
"$CXX" -std=c++11 -Wall -Wextra -Werror $NATIVE_DEFINES \
	-I"$ROOT" -I"$ROOT/public" -I"$ROOT/common" \
	"$ROOT/rust/tests/lzss_differential.cpp" "$TARGET_DIR/lzss_native.o" \
	"$TARGET_DIR/lzss_native_stub.o" \
	-L"$TARGET_DIR/debug" -lsource_abi "$RPATH_FLAG" \
	-o "$TARGET_DIR/lzss_differential"
"$TARGET_DIR/lzss_differential"
