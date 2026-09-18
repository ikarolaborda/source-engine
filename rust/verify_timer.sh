#!/bin/sh
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
BUILD_DIR=${1:-"$ROOT/build-rust-check"}
CXX=${CXX:-c++}
PLATFORM_DEFINES="-DPOSIX=1 -D_POSIX=1 -DPLATFORM_POSIX=1 -DGNUC -DPLATFORM_64BITS=1"
if [ "$(uname -s)" = "Darwin" ]; then
	PLATFORM_DEFINES="$PLATFORM_DEFINES -DOSX=1 -D_OSX=1"
fi

if [ ! -f "$BUILD_DIR/tier0/libtier0.dylib" ] && [ ! -f "$BUILD_DIR/tier0/libtier0.so" ]; then
	echo "tier0 library not found in $BUILD_DIR; configure and build Waf first" >&2
	exit 2
fi

"$CXX" -std=c++11 -Wall -Wextra -Werror \
	$PLATFORM_DEFINES \
	-I"$ROOT/public" "$ROOT/rust/tests/timer_smoke.cpp" \
	-L"$BUILD_DIR/tier0" -ltier0 -Wl,-rpath,"$BUILD_DIR/tier0" \
	-o "$BUILD_DIR/timer_smoke"
"$BUILD_DIR/timer_smoke"
