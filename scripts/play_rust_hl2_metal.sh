#!/bin/sh
set -eu

# Normal interactive play, not a test harness. Additional arguments can request
# a first-run resolution (-w 3840 -h 2160); otherwise use the saved video options.
SCRIPT_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
RUNTIME="$SCRIPT_ROOT/out-rust-allgames"
if [ ! -x "$RUNTIME/hl2_launcher" ] || [ ! -f "$RUNTIME/bin/libsource_abi.dylib" ] ||
   [ ! -f "$RUNTIME/hl2/gameinfo.txt" ]; then
	echo "Install the Rust-enabled HL2 build and content in $RUNTIME first." >&2
	exit 1
fi
cd "$RUNTIME"
unset SOURCE_CONTENT_ROOT SOURCE_HL2_CONTENT_ROOT
export DYLD_LIBRARY_PATH="$RUNTIME/bin"
export DYLD_FALLBACK_LIBRARY_PATH="$RUNTIME/bin:$RUNTIME/hl2/bin"
exec ./hl2_launcher -game hl2 -metal -fullscreen -novid \
	+rawinput_set_one_time 1 +m_rawinput 1 +m_customaccel 0 +m_filter 0 \
	+closecaption 1 +cc_subtitles 0 "$@"
