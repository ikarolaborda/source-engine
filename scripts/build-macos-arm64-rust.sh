#!/bin/sh
set -eu

# Build and install the transitional Rust-engine configuration for the first
# platform target of the port.  This is the configuration every live HL2 gate
# runs against, so CI has to keep its complete Waf graph linking.

git submodule init && git submodule update

if command -v brew >/dev/null 2>&1; then
	brew list sdl2 >/dev/null 2>&1 || brew install sdl2
fi

ARCH=$(uname -m)
if [ "$ARCH" != "arm64" ]; then
	echo "expected an arm64 host, found $ARCH" >&2
	exit 1
fi

# A Rust installed outside rustup, such as Homebrew's, takes precedence on PATH
# and ignores rust-toolchain.toml, which would build the engine's Rust library
# with a different compiler than CI uses.  Preferring rustup's shims keeps the
# pin in effect whatever the calling shell looks like; Waf still verifies the
# resulting versions against the pin and refuses a mismatch.
if [ -d "$HOME/.cargo/bin" ]; then
	PATH="$HOME/.cargo/bin:$PATH"
	export PATH
fi

BUILD_DIR=${BUILD_DIR:-build-rust-ci}
INSTALL_DIR=${INSTALL_DIR:-$(pwd)/out-rust-ci}

# Waf reads the build directory back out of its lock file, and --out on a build
# does not override it.  A lock named after this build directory therefore keeps
# the script building what it configured even when the tree already holds a
# developer's default lock, which otherwise silently redirects the build.
WAFLOCK=".lock-waf-$BUILD_DIR"
export WAFLOCK

python3 waf configure -T release --disable-warns --rust-engine \
	--prefix="$INSTALL_DIR" --out="$BUILD_DIR" "$@"
python3 waf build
python3 waf install

# The Rust process entry must be the installed launcher, and it must be a
# native arm64 image rather than a translated one.
LAUNCHER="$INSTALL_DIR/hl2_launcher"
if [ ! -x "$LAUNCHER" ]; then
	echo "install did not produce $LAUNCHER" >&2
	exit 1
fi
file "$LAUNCHER" | grep -q 'arm64' || {
	echo "$LAUNCHER is not an arm64 image" >&2
	exit 1
}
cmp -s "$LAUNCHER" "$BUILD_DIR/cargo-target/release/source-launcher" || {
	echo "$LAUNCHER is not the Cargo-built source-launcher" >&2
	exit 1
}

echo "macOS arm64 --rust-engine build and install verified in $INSTALL_DIR"
