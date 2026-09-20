#!/bin/sh
set -eu

# Parse the whole harness before running, so an ongoing gate is unaffected by
# later edits to this file.
main()
{

# A direct +map gate for any already-installed game. Content must already be
# linked into this isolated runtime; this script never prepares a Steam folder.
if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then
	echo "usage: $0 <installed-runtime> <game> <map> [timeout-seconds]" >&2
	exit 2
fi
RUNTIME=$(CDPATH= cd -- "$1" && pwd)
GAME=$2
MAP=$3
DURATION=${4:-240}
SCENARIO=${RUST_GAME_SCENARIO:-smoke}
FAILURE_ARGS=
SPLIT_ARGS=
case "${RUST_GAME_SPLIT_STARTUP:-0}" in
	0) ;;
	1) SPLIT_ARGS=-test_split_app_lifecycle ;;
	*) echo "RUST_GAME_SPLIT_STARTUP must be 0 or 1" >&2; exit 2 ;;
esac
RENDERER=${RUST_GAME_RENDERER:-metal}
case "$GAME:$MAP" in *[!a-zA-Z0-9_:-]*|'':*) exit 2 ;; esac
case "$DURATION" in ''|*[!0-9]*) exit 2 ;; esac
[ "$DURATION" -gt 0 ] || exit 2
case "$RENDERER" in
	metal) RENDER_ARGS=-metal ;;
	togl) RENDER_ARGS= ;;
	*) echo "RUST_GAME_RENDERER must be metal or togl" >&2; exit 2 ;;
esac
case "$SCENARIO" in
	init-failure)
		SCENARIO_FILE=game_smoke.txt
		STARTED=unused; COMPLETED=unused
		SOUND_ARGS=-nosound
		WIDTH=640; HEIGHT=480
		FAILURE_ARGS=-test_fail_shader_device_init
		;;
	smoke)
		SCENARIO_FILE=game_smoke.txt
		STARTED=RUST_GAME_SMOKE_LOADED
		COMPLETED=RUST_GAME_SMOKE_COMPLETED
		SOUND_ARGS=-nosound
		WIDTH=640; HEIGHT=480
		;;
	ep2-lipsync)
		[ "$GAME:$MAP" = "ep2:ep2_outland_02" ] || exit 2
		SCENARIO_FILE=ep2_lipsync.txt
		STARTED=RUST_EP2_LIPSYNC_STARTED
		COMPLETED=RUST_EP2_LIPSYNC_COMPLETED
		SOUND_ARGS=
		WIDTH=1280; HEIGHT=960
		;;
	*) echo "RUST_GAME_SCENARIO must be smoke, ep2-lipsync or init-failure" >&2; exit 2 ;;
esac
SCRIPT_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
sh "$SCRIPT_ROOT/scripts/prepare_rust_runtime_writes.sh" "$RUNTIME" "$GAME"
if [ ! -x "$RUNTIME/hl2_launcher" ] || [ ! -f "$RUNTIME/bin/libsource_abi.dylib" ] ||
	[ ! -f "$RUNTIME/$GAME/gameinfo.txt" ]; then
	echo "runtime must contain a Rust build and the selected gameinfo.txt" >&2
	exit 1
fi
for WRITABLE in cfg save screenshots downloadlists testscripts; do
	if [ -L "$RUNTIME/$GAME/$WRITABLE" ]; then
		echo "refusing writable symlink: $RUNTIME/$GAME/$WRITABLE" >&2
		exit 1
	fi
	mkdir -p "$RUNTIME/$GAME/$WRITABLE"
done
for WRITABLE in console.log demoheader.tmp gamestate.txt stats.txt videoconfig_mac.cfg voice_ban.dt glshaders.cfg; do
	if [ -L "$RUNTIME/$GAME/$WRITABLE" ]; then
		echo "refusing writable symlink: $RUNTIME/$GAME/$WRITABLE" >&2
		exit 1
	fi
done
RUN_DIR=$(mktemp -d "/tmp/source-rust-$GAME-$MAP-$RENDERER.XXXXXX")
mkdir -p "$RUN_DIR/frames"
if [ -f "$RUNTIME/engine.log" ]; then
	mv "$RUNTIME/engine.log" "$RUN_DIR/prior-engine.log"
fi
cp "$SCRIPT_ROOT/rust/tests/$SCENARIO_FILE" "$RUNTIME/$GAME/testscripts/rust_game_smoke.txt"
if [ "$SCENARIO" = ep2-lipsync ]; then
	mkdir -p "$RUN_DIR/prior-shots"
	for SHOT in "$RUNTIME/$GAME"/screenshots/rust_ep2_lip*.jpg; do
		if [ -f "$SHOT" ]; then mv "$SHOT" "$RUN_DIR/prior-shots/"; fi
	done
fi
cd "$RUNTIME"
export DYLD_LIBRARY_PATH="$RUNTIME/bin"
export DYLD_FALLBACK_LIBRARY_PATH="$RUNTIME/bin:$RUNTIME/$GAME/bin"
# Exercise the runtime's own links without relying on the old HL2-only override.
unset SOURCE_CONTENT_ROOT SOURCE_HL2_CONTENT_ROOT
export SOURCE_D3D9_SHOT_DIR="$RUN_DIR/frames"
export SOURCE_D3D9_SHOT_EVERY=60
echo "game=$GAME map=$MAP renderer=$RENDERER scenario=$SCENARIO evidence=$RUN_DIR"
./hl2_launcher -game "$GAME" $RENDER_ARGS $FAILURE_ARGS $SPLIT_ARGS -multirun -novid -nojoy $SOUND_ARGS \
	-windowed -w "$WIDTH" -h "$HEIGHT" -nomouse -nomousegrab -nomessagebox -console \
	+rawinput_set_one_time 1 +m_rawinput 0 +snd_mute_losefocus 0 \
	-testscript rust_game_smoke.txt +map "$MAP" > "$RUN_DIR/launcher.log" 2>&1 &
GAME_PID=$!
trap 'kill -TERM "$GAME_PID" 2>/dev/null || true' INT TERM EXIT
ELAPSED=0
while kill -0 "$GAME_PID" 2>/dev/null && [ "$ELAPSED" -lt "$DURATION" ]; do
	sleep 1
	ELAPSED=$((ELAPSED + 1))
done
TIMED_OUT=0
if kill -0 "$GAME_PID" 2>/dev/null; then
	TIMED_OUT=1
	sample "$GAME_PID" 3 -file "$RUN_DIR/timeout-sample.txt" >/dev/null 2>&1 || true
	kill -TERM "$GAME_PID" 2>/dev/null || true
	sleep 2
	kill -KILL "$GAME_PID" 2>/dev/null || true
fi
STATUS=0
wait "$GAME_PID" || STATUS=$?
trap - INT TERM EXIT
if [ -f "$RUNTIME/engine.log" ]; then
	cp "$RUNTIME/engine.log" "$RUN_DIR/engine.log"
fi
if [ "$SCENARIO" = init-failure ]; then
	# Launcher returns -1 for a failed session: shell exit 255, not SIGSEGV 139.
	if [ "$TIMED_OUT" -ne 0 ] || [ "$STATUS" -ne 255 ]; then
		echo "init-failure cleanup failed: timeout=$TIMED_OUT exit=$STATUS evidence=$RUN_DIR" >&2
		exit 1
	fi
	if [ -n "$SPLIT_ARGS" ]; then
		INNER_CLEANUP="Rust split app-system startup rolled back: stage 3, status 0"
		OUTER_CLEANUP="Rust split app-system cleanup complete: stage 8"
	else
		INNER_CLEANUP="Rust app-system group cleanup complete: stage 3, result -1"
		OUTER_CLEANUP="Rust app-system group cleanup complete: stage 8, result -1"
	fi
	for EXPECTED in \
		"Shader device initialization failure injected for lifecycle test" \
		"$INNER_CLEANUP" \
		"$OUTER_CLEANUP" \
		"Rust host session loop failed with status 8"; do
		if ! rg -q "$EXPECTED" "$RUN_DIR/launcher.log"; then
			echo "missing failure-path marker: $EXPECTED (evidence=$RUN_DIR)" >&2
			exit 1
		fi
	done
	echo "Rust injected-init-failure cleanup passed: $RENDERER (exit=$STATUS, $RUN_DIR)"
	return
fi
if [ -n "$SPLIT_ARGS" ]; then
	for MARKER in "Rust split app-system startup ready" "Rust split app-system cleanup complete: stage 8"; do
		COUNT=$(rg -c "$MARKER" "$RUN_DIR/launcher.log" || true)
		if [ "${COUNT:-0}" -ne 2 ]; then
			echo "expected 2 split lifecycle markers: $MARKER, got ${COUNT:-0} (evidence=$RUN_DIR)" >&2
			exit 1
		fi
	done
fi
if [ "$TIMED_OUT" -ne 0 ] || [ "$STATUS" -ne 0 ]; then
	echo "game smoke failed: timeout=$TIMED_OUT exit=$STATUS evidence=$RUN_DIR" >&2
	exit 1
fi
for EXPECTED in \
	"Rust GAME search mounts: [1-9][0-9]* from $GAME/gameinfo.txt" \
	"Rust BSP world: .*maps/$MAP.bsp" \
	"SV_ActivateServer: setting tickrate" \
	"Rust app-system group cleanup complete: stage 8, result" \
	"$STARTED" \
	"$COMPLETED"; do
	if ! rg -q "$EXPECTED" "$RUN_DIR/engine.log"; then
		echo "missing marker: $EXPECTED (evidence=$RUN_DIR)" >&2
		exit 1
	fi
done
if [ "${RUST_GAME_REQUIRE_PAK:-0}" = 1 ]; then
	for EXPECTED in "Rust archive range discovered:" "Rust pack index active:" "Rust filesystem ZIP/BSP mounts active:" "Rust filesystem ZIP/BSP read active:"; do
		if ! rg -q "$EXPECTED" "$RUN_DIR/engine.log"; then
			echo "missing archive ownership marker: $EXPECTED (evidence=$RUN_DIR)" >&2
			exit 1
		fi
	done
	if rg 'Rust pack (index|payload|wildcard|discovery) rejected:|Rust filesystem (read path synchronization failed|ZIP/BSP mount rejected|rejected read)' "$RUN_DIR/engine.log"; then
		echo "archive ownership regression (evidence=$RUN_DIR)" >&2
		exit 1
	fi
fi
if [ "$RENDERER" = metal ]; then
	if ! rg -q "d3d9metal: device .*layer attached" "$RUN_DIR/launcher.log" ||
		rg -i 'd3d9metal:.*(failed|error|unsupported|could not|panic)' "$RUN_DIR/launcher.log"; then
		echo "Metal device validation failed (evidence=$RUN_DIR)" >&2
		exit 1
	fi
	set -- "$RUN_DIR"/frames/frame-*.bmp
	if [ "$SCENARIO" = smoke ] && [ ! -s "$1" ]; then
		echo "no Metal frames captured (evidence=$RUN_DIR)" >&2
		exit 1
	fi
else
	if ! rg -q 'LoadLibrary:.*libshaderapidx9.dylib' "$RUN_DIR/launcher.log"; then
		echo "ToGL shader API was not loaded (evidence=$RUN_DIR)" >&2
		exit 1
	fi
fi
if [ "$SCENARIO" = ep2-lipsync ]; then
	SHOT_COUNT=0
	for SHOT in "$RUNTIME/$GAME"/screenshots/rust_ep2_lip*.jpg; do
		if [ -s "$SHOT" ]; then
			cp "$SHOT" "$RUN_DIR/frames/"
			SHOT_COUNT=$((SHOT_COUNT + 1))
		fi
	done
	if [ "$SHOT_COUNT" -ne 40 ]; then
		echo "expected 40 speech frames, got $SHOT_COUNT (evidence=$RUN_DIR)" >&2
		exit 1
	fi
	for EXPECTED in RUST_LIPSYNC_PHONEMES_READY RUST_LIPSYNC_VISEME_ACTIVE \
		RUST_LIPSYNC_RENDER_FLEX_ACTIVE RUST_LIPSYNC_CLOCK_STARTED; do
		if ! rg -q "$EXPECTED" "$RUN_DIR/engine.log"; then
			echo "missing speech marker: $EXPECTED (evidence=$RUN_DIR)" >&2
			exit 1
		fi
	done
	for STAGE in VISEME:magnitude RENDER:total; do
		MARKER="RUST_LIPSYNC_${STAGE%%:*}_SAMPLE"
		FIELD=${STAGE##*:}
		DISTINCT=$(rg -o "$MARKER .*$FIELD=([0-9.]+)" -r '$1' "$RUN_DIR/engine.log" | sort -u | grep -c .)
		if [ "$DISTINCT" -lt 5 ]; then
			echo "$MARKER has only $DISTINCT distinct values (evidence=$RUN_DIR)" >&2
			exit 1
		fi
	done
fi
echo "Rust direct-map $RENDERER $SCENARIO passed: $GAME/$MAP ($RUN_DIR)"
}

main "$@"
