#!/bin/sh
set -eu

# Makes a playable folder out of a --rust-engine build, beside an existing
# Half-Life 2 folder rather than over it.
#
# The new folder holds this build's binaries and its own settings and saves.
# The game's content, which is gigabytes of archives this build only reads, is
# linked from the existing folder rather than copied. Nothing in the existing
# folder is written to: its binaries keep working as they did, and its saves
# are copied once so that a fault in this build cannot reach the originals.
#
#   install_macos_metal_runtime.sh <installed-build> <game-folder> <new-folder>
#
# <installed-build> is what scripts/build-macos-arm64-rust.sh installs into,
# out-rust-ci by default. Run it again after a rebuild to refresh the binaries;
# settings and saves already in <new-folder> are left alone.

usage()
{
	echo "usage: $0 <installed-build> <game-folder> <new-folder>" >&2
	exit 2
}

[ "$#" -eq 3 ] || usage

BUILD=$(CDPATH= cd -- "$1" && pwd)
GAME=$(CDPATH= cd -- "$2" && pwd)
mkdir -p "$3"
DEST=$(CDPATH= cd -- "$3" && pwd)

if [ ! -x "$BUILD/hl2_launcher" ] || [ ! -f "$BUILD/bin/libshaderapimetal.dylib" ]; then
	echo "not an installed --rust-engine build with the Metal shader API: $BUILD" >&2
	exit 1
fi
if [ ! -d "$GAME/hl2" ] || [ ! -d "$GAME/platform" ]; then
	echo "game folder is missing hl2/ or platform/: $GAME" >&2
	exit 1
fi
if [ "$DEST" = "$GAME" ] || [ "$DEST" = "$BUILD" ]; then
	echo "the new folder must not be the game folder or the build" >&2
	exit 1
fi

# Binaries. --delete keeps a refreshed folder from carrying libraries a newer
# build no longer produces, which the engine would otherwise still find.
mkdir -p "$DEST/bin" "$DEST/hl2/bin"
rsync -a --delete "$BUILD/bin/" "$DEST/bin/"
rsync -a --delete "$BUILD/hl2/bin/" "$DEST/hl2/bin/"
cp -f "$BUILD/hl2_launcher" "$DEST/hl2_launcher"
chmod 755 "$DEST/hl2_launcher"

# Content, linked. Everything the engine writes is left out of this list and
# handled below, so that no write follows a link back into the game folder.
for SOURCE_PATH in "$GAME/hl2"/*
do
	NAME=$(basename "$SOURCE_PATH")
	case "$NAME" in
		bin|cfg|save|screenshots|downloadlists|testscripts|console.log|demoheader.tmp|gamestate.txt|stats.txt|videoconfig_mac.cfg|voice_ban.dt|glshaders.cfg|*.sound.cache)
			continue
			;;
	esac
	if [ ! -e "$DEST/hl2/$NAME" ] && [ ! -L "$DEST/hl2/$NAME" ]; then
		ln -s "$SOURCE_PATH" "$DEST/hl2/$NAME"
	fi
done
if [ ! -e "$DEST/platform" ] && [ ! -L "$DEST/platform" ]; then
	cp -RL "$GAME/platform" "$DEST/platform"
fi
SCRIPT_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
sh "$SCRIPT_ROOT/scripts/prepare_rust_runtime_writes.sh" "$DEST" hl2
if [ ! -e "$DEST/steam_appid.txt" ] && [ -f "$GAME/steam_appid.txt" ]; then
	cp "$GAME/steam_appid.txt" "$DEST/steam_appid.txt"
fi

# Settings, saves and chapter progress, copied once and then this folder's own.
# The video settings are not copied: they describe a display mode in points and
# a multisampling level that mean something else, or nothing, to this renderer.
if [ ! -d "$DEST/hl2/cfg" ]; then
	cp -R "$GAME/hl2/cfg" "$DEST/hl2/cfg"
fi
if [ ! -d "$DEST/hl2/save" ]; then
	if [ -d "$GAME/hl2/save" ]; then
		cp -R "$GAME/hl2/save" "$DEST/hl2/save"
	else
		mkdir -p "$DEST/hl2/save"
	fi
fi
mkdir -p "$DEST/hl2/screenshots" "$DEST/hl2/downloadlists"
if [ ! -e "$DEST/hl2/gamestate.txt" ] && [ -f "$GAME/hl2/gamestate.txt" ]; then
	cp "$GAME/hl2/gamestate.txt" "$DEST/hl2/gamestate.txt"
fi

# Full screen at the display's own pixel count, through Metal. +exec trainer is
# harmless where there is no trainer.cfg: the engine says so and carries on.
LAUNCH_ARGS='-game hl2 -metal -fullscreen -nativeres +exec trainer'

cat > "$DEST/run.sh" <<EOF
#!/bin/sh
# Half-Life 2 through the Rust host and Direct3D 9 on Metal.
DIR=\$(cd "\$(dirname "\$0")" && pwd)
cd "\$DIR" || exit 1
export DYLD_LIBRARY_PATH="\$DIR/bin"
export DYLD_FALLBACK_LIBRARY_PATH="\$DIR/bin:\$DIR/hl2/bin"
exec ./hl2_launcher $LAUNCH_ARGS "\$@"
EOF
chmod 755 "$DEST/run.sh"

# A bundle so the game starts from Finder or the Dock. Its script runs the
# launcher in the folder rather than a copy inside the bundle, because the
# engine finds its libraries and content relative to the launcher it was
# started as.
APP="$DEST/Half-Life 2 Metal.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
if [ -f "$GAME/hl2/resource/game.icns" ]; then
	cp -f "$GAME/hl2/resource/game.icns" "$APP/Contents/Resources/game.icns"
fi
printf 'APPL????' > "$APP/Contents/PkgInfo"
cat > "$APP/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>Half-Life 2 Metal</string>
	<key>CFBundleExecutable</key>
	<string>Half-Life 2 Metal</string>
	<key>CFBundleIconFile</key>
	<string>game.icns</string>
	<key>CFBundleIdentifier</key>
	<string>com.ikarolaborda.hl2.metal</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>Half-Life 2 Metal</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>1.0</string>
	<key>CFBundleVersion</key>
	<string>1</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.action-games</string>
	<key>LSMinimumSystemVersion</key>
	<string>12.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
EOF
cat > "$APP/Contents/MacOS/Half-Life 2 Metal" <<EOF
#!/bin/sh
# Started from Finder, so there is no terminal: what the engine prints goes to
# launcher.log beside the game.
BUNDLE=\$(cd "\$(dirname "\$0")/../.." && pwd)
GAME=\$(dirname "\$BUNDLE")
if [ ! -x "\$GAME/hl2_launcher" ]; then
	GAME="$DEST"
fi
if [ ! -x "\$GAME/hl2_launcher" ]; then
	osascript -e 'display alert "Half-Life 2 Metal" message "Could not find hl2_launcher. Keep this app inside its game folder." as critical' >/dev/null 2>&1
	exit 1
fi
cd "\$GAME" || exit 1
export DYLD_LIBRARY_PATH="\$GAME/bin"
export DYLD_FALLBACK_LIBRARY_PATH="\$GAME/bin:\$GAME/hl2/bin"
exec "\$GAME/hl2_launcher" $LAUNCH_ARGS "\$@" > "\$GAME/launcher.log" 2>&1
EOF
chmod 755 "$APP/Contents/MacOS/Half-Life 2 Metal"

echo "installed into $DEST"
echo "start it with \"$DEST/run.sh\" or the app inside it"
