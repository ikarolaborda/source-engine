#!/bin/sh
set -eu

# Parse the complete gate before executing it. Editing this harness while its
# long-running native child is active must not shift the shell's input cursor.
main()
{
ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
[ "$#" -eq 1 ] || { echo "usage: $0 <installed-rust-runtime>" >&2; exit 2; }
RUNTIME=$(CDPATH= cd -- "$1" && pwd)
[ "$(uname -s)" = Darwin ] || { echo "native filesystem gate currently requires macOS" >&2; exit 2; }
TEST_DIR=$(mktemp -d /tmp/source-native-pak.XXXXXX)
# Check actual implementation retirement, not only output compatibility.
nm -C "$RUNTIME/bin/libfilesystem_stdio.dylib" > "$TEST_DIR/filesystem-symbols.txt"
if grep -E 'CZipPackFileHandle|CLZMAZipPackFileHandle|CZipPackFile::ReadFromPack' "$TEST_DIR/filesystem-symbols.txt"; then
  echo "native pack payload implementation remains in Rust filesystem" >&2
  exit 1
fi
grep -q 'CRustPackFileHandle::Read' "$TEST_DIR/filesystem-symbols.txt"
nm -u "$RUNTIME/bin/libfilesystem_stdio.dylib" > "$TEST_DIR/filesystem-imports.txt"
grep -q 'source_rust_bridge_pak_index_open_archive' "$TEST_DIR/filesystem-imports.txt"
grep -q 'source_rust_bridge_find_first_pak' "$TEST_DIR/filesystem-imports.txt"
grep -q 'source_rust_bridge_find_pack_candidates' "$TEST_DIR/filesystem-imports.txt"
grep -q 'source_rust_bridge_read_path_add_pak_index' "$TEST_DIR/filesystem-imports.txt"
grep -q 'source_rust_bridge_read_path_matches' "$TEST_DIR/filesystem-imports.txt"
for symbol in mount_table_create mount_table_insert mount_table_get mount_table_remove mount_table_clear mount_table_snapshot mount_table_destroy mount_store_id_next; do
  grep -q "source_rust_bridge_$symbol" "$TEST_DIR/filesystem-imports.txt"
done
if grep -q 'g_iNextSearchPathID' "$TEST_DIR/filesystem-symbols.txt"; then
  echo "native store-ID counter remains in Rust filesystem" >&2
  exit 1
fi
for symbol in search_plan_create search_plan_next search_visits_create search_visits_mark search_state_destroy; do
  grep -q "source_rust_bridge_$symbol" "$TEST_DIR/filesystem-imports.txt"
done
if grep -E -q 'source_rust_bridge_(pak_index_(open_file|create)|read_path_add_pak_flags)' "$TEST_DIR/filesystem-imports.txt"; then
  echo "native filesystem still supplies archive byte ranges or reopens mounted packs" >&2
  exit 1
fi
# Link the bridge's exported ABI, which owns the live filesystem context. Linking
# a second libsource_abi instance would create a separate handle registry.
"${CXX:-clang++}" -std=c++11 -DPOSIX -D_OSX -DOSX -DGNUC -DSUPPORT_PACKED_STORE -UNDEBUG \
  -Wno-inconsistent-missing-override \
  -I"$ROOT/public" -I"$ROOT/public/tier0" -I"$ROOT/public/tier1" -I"$ROOT/appframework" \
  "$ROOT/rust/tests/pak_native_read.cpp" -L"$RUNTIME/bin" \
  -ltier0 -lrust_engine_bridge -Wl,-rpath,"$RUNTIME/bin" \
  -o "$TEST_DIR/pak_native_read"
export DYLD_LIBRARY_PATH="$RUNTIME/bin"
export SOURCE_PAK_NATIVE_FS="$TEST_DIR/pak_native_read"
export SOURCE_PAK_NATIVE_FS_LIBRARY="$RUNTIME/bin/libfilesystem_stdio.dylib"
export SOURCE_PAK_NATIVE_LZMA="$TEST_DIR/pak_lzma_oracle.dylib"
"${CC:-clang}" -shared -fPIC -D_7ZIP_ST \
  "$ROOT/utils/lzma/C/LzmaLib.c" "$ROOT/utils/lzma/C/LzmaEnc.c" \
  "$ROOT/utils/lzma/C/LzmaDec.c" "$ROOT/utils/lzma/C/LzFind.c" \
  "$ROOT/utils/lzma/C/Alloc.c" -o "$SOURCE_PAK_NATIVE_LZMA"
python3 "$ROOT/rust/tests/pak_differential.py" "$RUNTIME/bin/libsource_abi.dylib"
}

# Parse exit together with the invocation, so success never resumes reading a
# potentially edited script at the old byte offset.
main "$@"; exit
