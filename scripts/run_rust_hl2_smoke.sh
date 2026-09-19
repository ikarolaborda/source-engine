#!/bin/sh
set -eu

usage()
{
	echo "usage: $0 <installed-runtime-root> <content-root> [seconds] [smoke|save-demo|ten-minute|intro-lipsync|physics|hud-audio|soak]" >&2
	exit 2
}

# The whole run lives in a function so the shell parses this file completely
# before executing any of it. The soak scenario runs for over an hour, and a
# shell that kept reading the file would misparse the rest of the run if the
# file were edited in the meantime.
main()
{

[ "$#" -ge 2 ] && [ "$#" -le 4 ] || usage

RUNTIME_ROOT=$1
CONTENT_ROOT=$2
DURATION_SECONDS=${3:-30}
SCENARIO=${4:-smoke}
SCRIPT_ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)

case "$DURATION_SECONDS" in
	''|*[!0-9]*) usage ;;
esac
[ "$DURATION_SECONDS" -gt 0 ] || usage
case "$SCENARIO" in
	smoke|save-demo|ten-minute|intro-lipsync|physics|hud-audio|soak) ;;
	*) usage ;;
esac
if [ "$SCENARIO" = "ten-minute" ] && [ "$DURATION_SECONDS" -lt 620 ]; then
	echo "ten-minute scenario requires a timeout of at least 620 seconds" >&2
	exit 2
fi
if [ "$SCENARIO" = "soak" ] && [ "$DURATION_SECONDS" -lt 3660 ]; then
	echo "soak scenario requires a timeout of at least 3660 seconds" >&2
	exit 2
fi

if [ ! -x "$RUNTIME_ROOT/hl2_launcher" ] || [ ! -f "$RUNTIME_ROOT/bin/liblauncher.dylib" ]; then
	echo "runtime root is not an installed build: $RUNTIME_ROOT" >&2
	exit 1
fi

# The run changes directory into the runtime root, so both roots are made
# absolute first. A relative root would otherwise be re-resolved against
# itself and the logs would be written nowhere.
RUNTIME_ROOT=$(CDPATH= cd -- "$RUNTIME_ROOT" && pwd)
CONTENT_ROOT=$(CDPATH= cd -- "$CONTENT_ROOT" && pwd)
if [ ! -d "$CONTENT_ROOT/hl2" ] || [ ! -d "$CONTENT_ROOT/platform" ]; then
	echo "content root is missing hl2/platform: $CONTENT_ROOT" >&2
	exit 1
fi
if [ "$RUNTIME_ROOT" = "$CONTENT_ROOT" ]; then
	echo "runtime and content roots must be different" >&2
	exit 1
fi

mkdir -p "$RUNTIME_ROOT/hl2"
for SOURCE_PATH in "$CONTENT_ROOT/hl2"/*
do
	NAME=$(basename "$SOURCE_PATH")
	case "$NAME" in
		bin|cfg|save|screenshots|downloadlists|testscripts|console.log|demoheader.tmp|gamestate.txt|stats.txt|videoconfig_mac.cfg|voice_ban.dt)
			continue
			;;
	esac
	DESTINATION="$RUNTIME_ROOT/hl2/$NAME"
	if [ ! -e "$DESTINATION" ] && [ ! -L "$DESTINATION" ]; then
		ln -s "$SOURCE_PATH" "$DESTINATION"
	fi
done

for WRITABLE_PATH in cfg save screenshots downloadlists
do
	if [ -L "$RUNTIME_ROOT/hl2/$WRITABLE_PATH" ]; then
		echo "refusing writable path symlink: $RUNTIME_ROOT/hl2/$WRITABLE_PATH" >&2
		exit 1
	fi
done
if [ ! -d "$RUNTIME_ROOT/hl2/cfg" ]; then
	cp -R "$CONTENT_ROOT/hl2/cfg" "$RUNTIME_ROOT/hl2/cfg"
fi
mkdir -p "$RUNTIME_ROOT/hl2/save" "$RUNTIME_ROOT/hl2/screenshots" \
	"$RUNTIME_ROOT/hl2/downloadlists"

for WRITABLE_FILE in console.log demoheader.tmp gamestate.txt stats.txt videoconfig_mac.cfg voice_ban.dt
do
	DESTINATION="$RUNTIME_ROOT/hl2/$WRITABLE_FILE"
	if [ -L "$DESTINATION" ]; then
		echo "refusing writable file symlink: $DESTINATION" >&2
		exit 1
	fi
	if [ ! -e "$DESTINATION" ] && [ -f "$CONTENT_ROOT/hl2/$WRITABLE_FILE" ]; then
		cp "$CONTENT_ROOT/hl2/$WRITABLE_FILE" "$DESTINATION"
	fi
done

if [ ! -e "$RUNTIME_ROOT/platform" ] && [ ! -L "$RUNTIME_ROOT/platform" ]; then
	ln -s "$CONTENT_ROOT/platform" "$RUNTIME_ROOT/platform"
fi
if [ ! -e "$RUNTIME_ROOT/steam_appid.txt" ] && [ -f "$CONTENT_ROOT/steam_appid.txt" ]; then
	cp "$CONTENT_ROOT/steam_appid.txt" "$RUNTIME_ROOT/steam_appid.txt"
fi

PRIOR_LOGS=$(mktemp -d /tmp/source-rust-prior-logs.XXXXXX)
for LOG_NAME in engine.log runtime-smoke.log
do
	if [ -f "$RUNTIME_ROOT/$LOG_NAME" ]; then
		mv "$RUNTIME_ROOT/$LOG_NAME" "$PRIOR_LOGS/$LOG_NAME"
	fi
done

if [ "$SCENARIO" != "smoke" ]; then
	if [ -L "$RUNTIME_ROOT/hl2/testscripts" ]; then
		echo "refusing test-script path symlink: $RUNTIME_ROOT/hl2/testscripts" >&2
		exit 1
	fi
	mkdir -p "$RUNTIME_ROOT/hl2/testscripts"
	if [ "$SCENARIO" = "save-demo" ]; then
		cp "$SCRIPT_ROOT/rust/tests/hl2_acceptance.txt" \
			"$RUNTIME_ROOT/hl2/testscripts/rust_port_acceptance.txt"
	elif [ "$SCENARIO" = "intro-lipsync" ]; then
		cp "$SCRIPT_ROOT/rust/tests/hl2_intro_lipsync.txt" \
			"$RUNTIME_ROOT/hl2/testscripts/rust_port_intro_lipsync.txt"
	elif [ "$SCENARIO" = "physics" ]; then
		cp "$SCRIPT_ROOT/rust/tests/hl2_physics_interaction.txt" \
			"$RUNTIME_ROOT/hl2/testscripts/rust_port_physics.txt"
	elif [ "$SCENARIO" = "hud-audio" ]; then
		cp "$SCRIPT_ROOT/rust/tests/hl2_hud_audio.txt" \
			"$RUNTIME_ROOT/hl2/testscripts/rust_port_hud_audio.txt"
	elif [ "$SCENARIO" = "soak" ]; then
		cp "$SCRIPT_ROOT/rust/tests/hl2_soak.txt" \
			"$RUNTIME_ROOT/hl2/testscripts/rust_port_soak.txt"
	else
		cp "$SCRIPT_ROOT/rust/tests/hl2_ten_minute.txt" \
			"$RUNTIME_ROOT/hl2/testscripts/rust_port_ten_minute.txt"
	fi
fi
if [ "$SCENARIO" = "save-demo" ]; then
	for ARTIFACT in \
		"$RUNTIME_ROOT/hl2/rust_port_acceptance.dem" \
		"$RUNTIME_ROOT/hl2/save/rust_port_acceptance.sav"
	do
		if [ -f "$ARTIFACT" ]; then
			mv "$ARTIFACT" "$PRIOR_LOGS/$(basename "$ARTIFACT").previous"
		fi
	done
fi

cd "$RUNTIME_ROOT"
export DYLD_LIBRARY_PATH="$RUNTIME_ROOT/bin"
export DYLD_FALLBACK_LIBRARY_PATH="$RUNTIME_ROOT/bin:$RUNTIME_ROOT/hl2/bin"
export SOURCE_HL2_CONTENT_ROOT="$CONTENT_ROOT"

CAFFEINATE_PID=
if [ "$(uname -s)" = "Darwin" ] && command -v caffeinate >/dev/null 2>&1; then
	# The legacy ToGL adapter reports no renderer while the built-in display is
	# asleep. Wake it and keep it available for this bounded graphical run.
	caffeinate -u -t 1
	caffeinate -d -t $((DURATION_SECONDS + 60)) &
	CAFFEINATE_PID=$!
fi

# None of these runs are interactive, so none of them may capture the pointer:
# a gate that takes the mouse away makes the machine unusable for as long as it
# runs. `-nomousegrab` alone is not enough, because the grab is skipped only
# when raw input is off and macOS defaults it on, and
# `rawinput_set_one_time` otherwise forces raw input back on once per user.
NO_GRAB="-nomousegrab"
NO_GRAB_CVARS="+rawinput_set_one_time 1 +m_rawinput 0"

# The scenarios that assert on sound have to keep the audio device, the rest
# run silent so they neither need a device nor make noise.
#
# Keeping the device is not enough on its own. On POSIX the mixer drops every
# channel while the game is not the foreground application, and an unattended
# run never is, so sounds start and are then discarded before a sample of them
# is mixed. Nothing says so in the log, and the effects are easy to read as
# engine faults: a voice channel whose position never advances leaves lip-sync
# stuck on its first phoneme while the scene's flex tracks keep the face
# moving. Turning the muting off makes these runs hear what an attended
# session hears.
case "$SCENARIO" in
	intro-lipsync|hud-audio|soak) SOUND=""; SOUND_CVARS="+snd_mute_losefocus 0" ;;
	*) SOUND="-nosound"; SOUND_CVARS="" ;;
esac

case "$SCENARIO" in
	save-demo) TESTSCRIPT="-testscript rust_port_acceptance.txt" ;;
	ten-minute) TESTSCRIPT="-testscript rust_port_ten_minute.txt" ;;
	intro-lipsync) TESTSCRIPT="-testscript rust_port_intro_lipsync.txt" ;;
	physics) TESTSCRIPT="-testscript rust_port_physics.txt" ;;
	hud-audio) TESTSCRIPT="-testscript rust_port_hud_audio.txt" ;;
	soak) TESTSCRIPT="-testscript rust_port_soak.txt" ;;
	*) TESTSCRIPT="" ;;
esac

# `-nomessagebox` above is what keeps a failed run diagnosable. Every startup
# error the engine raises ends in a modal dialog, and an unattended run has
# nobody to dismiss one, so without it any error at all reports as a hang and
# the reason for it never reaches a log. With it the reason goes to stderr and
# the run fails on the marker it actually missed.

# Extra arguments for bisecting a platform subsystem against this harness,
# such as `-snd_openal` to take the AudioQueue device out of the picture.
# Running the engine by hand instead is not equivalent: the harness is what
# arranges the content, writable paths, and log rotation the engine needs.
EXTRA_ARGS=${EXTRA_LAUNCHER_ARGS:-}

# Unquoted on purpose: these hold several arguments each.
# shellcheck disable=SC2086
./hl2_launcher -game hl2 -novid $SOUND -nojoy -windowed -w 640 -h 480 $NO_GRAB \
	-console -nomessagebox $NO_GRAB_CVARS $SOUND_CVARS $EXTRA_ARGS \
	+maps d1_trainstation_01 $TESTSCRIPT +map d1_trainstation_01 \
	> "$RUNTIME_ROOT/runtime-smoke.log" 2>&1 &
GAME_PID=$!

ELAPSED=0
SERVER_ACTIVATED_AT=
# Runs have been observed hanging after the Rust host reports `content ready`
# and before the C++ engine writes its log at all, then being killed on the
# timeout, which destroys the only evidence of where they stopped. Sampling
# the threads once, while the process is still hung, turns the next occurrence
# into a stack instead of another lost run. The threshold is well past a cold
# start, which reaches the log in a couple of seconds.
STALL_SAMPLE="$RUNTIME_ROOT/startup-stall-sample.txt"
STALL_THRESHOLD=${STARTUP_STALL_SECONDS:-45}
STALL_SAMPLED=0
rm -f "$STALL_SAMPLE"
while [ "$ELAPSED" -lt "$DURATION_SECONDS" ] && kill -0 "$GAME_PID" 2>/dev/null
do
	sleep 1
	ELAPSED=$((ELAPSED + 1))
	if [ -z "$SERVER_ACTIVATED_AT" ] && [ -f "$RUNTIME_ROOT/engine.log" ] &&
		rg -q "SV_ActivateServer: setting tickrate" "$RUNTIME_ROOT/engine.log"; then
		SERVER_ACTIVATED_AT=$ELAPSED
	fi
	if [ "$STALL_SAMPLED" -eq 0 ] && [ "$ELAPSED" -ge "$STALL_THRESHOLD" ] &&
		[ ! -f "$RUNTIME_ROOT/engine.log" ]; then
		STALL_SAMPLED=1
		echo "no engine log after ${ELAPSED}s; sampling $GAME_PID into $STALL_SAMPLE" >&2
		sample "$GAME_PID" 4 -file "$STALL_SAMPLE" >/dev/null 2>&1 || true
	fi
done

# A cold shader cache can consume the entire requested smoke duration. Give a
# bounded startup grace before deciding that activation failed, then do not
# terminate the renderer immediately after activation while its asynchronous
# texture workers are still starting up.
if [ "$SCENARIO" = "smoke" ] && [ -z "$SERVER_ACTIVATED_AT" ]; then
	STARTUP_DEADLINE=$((DURATION_SECONDS + 30))
	while [ "$ELAPSED" -lt "$STARTUP_DEADLINE" ] && kill -0 "$GAME_PID" 2>/dev/null
	do
		sleep 1
		ELAPSED=$((ELAPSED + 1))
		if [ -f "$RUNTIME_ROOT/engine.log" ] &&
			rg -q "SV_ActivateServer: setting tickrate" "$RUNTIME_ROOT/engine.log"; then
			SERVER_ACTIVATED_AT=$ELAPSED
			break
		fi
	done
fi
if [ "$SCENARIO" = "smoke" ] && [ -n "$SERVER_ACTIVATED_AT" ]; then
	ACTIVE_SECONDS=$((ELAPSED - SERVER_ACTIVATED_AT))
	while [ "$ACTIVE_SECONDS" -lt 15 ] && kill -0 "$GAME_PID" 2>/dev/null
	do
		sleep 1
		ELAPSED=$((ELAPSED + 1))
		ACTIVE_SECONDS=$((ELAPSED - SERVER_ACTIVATED_AT))
	done
fi

REACHED_DURATION=0
if kill -0 "$GAME_PID" 2>/dev/null; then
	REACHED_DURATION=1
	kill -TERM "$GAME_PID"
fi
wait "$GAME_PID" || true
if [ -n "$CAFFEINATE_PID" ]; then
	kill "$CAFFEINATE_PID" 2>/dev/null || true
	wait "$CAFFEINATE_PID" 2>/dev/null || true
fi

RUNTIME_LOG="$RUNTIME_ROOT/runtime-smoke.log"
ENGINE_LOG="$RUNTIME_ROOT/engine.log"
if [ "$SCENARIO" = "smoke" ] && [ "$REACHED_DURATION" -ne 1 ]; then
	echo "runtime smoke exited before the requested ${DURATION_SECONDS}s duration (after ${ELAPSED}s)" >&2
	exit 1
fi
if [ ! -f "$ENGINE_LOG" ]; then
	echo "runtime smoke did not create the engine log" >&2
	if [ -s "$STALL_SAMPLE" ]; then
		echo "threads were sampled while it was hung: $STALL_SAMPLE" >&2
		# The topmost frames of the busiest thread are what distinguish a wait
		# on a lock, a device, or the filesystem, so they are echoed rather
		# than left for someone to go looking for.
		rg -m 1 -A 12 "^  Thread_" "$STALL_SAMPLE" >&2 || true
	fi
	exit 1
fi
for EXPECTED in \
	"Rust process entry active:" \
	"launcher context initialized" \
	"Rust console state: active generation 0" \
	"RUST_COMMAND_QUEUE_ACTIVE" \
	"Rust host phase: launcher ready" \
	"Rust executable base:" \
	"Rust GAME search mounts: 5 VPKs before loose content" \
	"Rust VPK read: CRC-validated" \
	"Rust filesystem read paths synchronized:" \
	"Rust filesystem legacy pack precedence guard synchronized:" \
	"Rust filesystem metadata active:" \
	"Rust filesystem path resolution active:" \
	"Rust filesystem wildcard search active:" \
	"Rust filesystem read active:" \
	"Rust filesystem read handle active:" \
	"Rust filesystem write path active:" \
	"Rust filesystem write handle active:" \
	"Rust filesystem directory creation active:" \
	"Rust net channel sequencing active:" \
	"Rust net channel outgoing sequence active:" \
	"Rust net channel incoming decisions active:" \
	"Rust net packet outgoing header active:" \
	"Rust net packet header active:" \
	"Rust network string tables active:" \
	"Rust datatable schema active:" \
	"Rust server class IDs active:" \
	"Rust snapshot state active:" \
	"Rust scene cache:" \
	"Rust BSP world:" \
	"Rust BSP world synchronized:" \
	"Rust host phase: content ready" \
	"Rust host phase: legacy engine running" \
	"Rust host frame pacing active:" \
	"Rust host tick scheduler active:" \
	"Rust host operation: new-game d1_trainstation_01" \
	"Rust host frame loop:" \
	"Rust host sessions:" \
	"Rust host phase: shutting down" \
	"Rust host phase: stopped"
do
	if ! rg -q "$EXPECTED" "$RUNTIME_LOG" "$ENGINE_LOG"; then
		echo "runtime smoke missing marker: $EXPECTED" >&2
		exit 1
	fi
done
if ! rg -q "SV_ActivateServer: setting tickrate" "$ENGINE_LOG"; then
	echo "runtime smoke did not activate the requested map server" >&2
	exit 1
fi
if ! rg -q ">>> Engine closed" "$ENGINE_LOG"; then
	echo "runtime smoke did not shut down cleanly" >&2
	exit 1
fi

if [ "$SCENARIO" = "save-demo" ]; then
	for ARTIFACT in \
		"$RUNTIME_ROOT/hl2/rust_port_acceptance.dem" \
		"$RUNTIME_ROOT/hl2/save/rust_port_acceptance.sav"
	do
		if [ ! -s "$ARTIFACT" ]; then
			echo "acceptance scenario did not create artifact: $ARTIFACT" >&2
			exit 1
		fi
	done
	cargo run --quiet --release \
		--manifest-path "$SCRIPT_ROOT/Cargo.toml" \
		-p source-content-audit -- --loose-only "$RUNTIME_ROOT"
	for EXPECTED in \
		"RUST_ACCEPTANCE_MOVEMENT_BEFORE" \
		"RUST_ACCEPTANCE_MOVEMENT_AFTER" \
		"RUST_ACCEPTANCE_SIMULATION_REPORT" \
		"Class: npc_" \
		"prop_physics -" \
		"Recording to rust_port_acceptance.dem" \
		"Completed demo, recording time" \
		"Rust demo validated: rust_port_acceptance.dem" \
		"Rust demo validated:.*[1-9][0-9]* string table sections, [1-9][0-9]* string table entries" \
		"Rust save validated: save/rust_port_acceptance.sav" \
		"Rust filesystem directory query active: save" \
		"Loading game from" \
		"Rust host operation: change-level-mp d1_trainstation_02" \
		"Rust BSP world synchronized:.*maps/d1_trainstation_02.bsp" \
		"Rust entity delta classification active:" \
		"Rust delta header encoding active:" \
		"Rust demo header encoding active:" \
		"Rust string table encoding active:" \
		"Rust map state encoded:" \
		"Rust save container encoded:" \
		"Playing demo from rust_port_acceptance.dem"
	do
		if ! rg -q "$EXPECTED" "$RUNTIME_LOG" "$ENGINE_LOG"; then
			echo "acceptance scenario missing marker: $EXPECTED" >&2
			exit 1
		fi
	done
	# Both stay on only while Rust and the native writer agree about every
	# entity, so any fallback means they diverged.
	for FALLBACK in \
		"Rust entity delta classification disabled" \
		"Rust delta header encoding disabled" \
		"Rust demo header encoding disabled" \
		"Rust string table encoding disabled" \
		"Rust map state encoding disabled" \
		"Rust save container encoding disabled"
	do
		if rg -q "$FALLBACK" "$RUNTIME_LOG" "$ENGINE_LOG"; then
			echo "acceptance scenario fell back: $FALLBACK" >&2
			rg -n "$FALLBACK" "$RUNTIME_LOG" "$ENGINE_LOG" >&2
			exit 1
		fi
	done
	MOVEMENT_BEFORE=$(awk '
		/RUST_ACCEPTANCE_MOVEMENT_BEFORE/ { waiting = 1 }
		waiting && /setpos_exact / {
			line = $0
			sub(/^.*setpos_exact /, "", line)
			sub(/;.*/, "", line)
			print line
			exit
		}' "$ENGINE_LOG")
	MOVEMENT_AFTER=$(awk '
		/RUST_ACCEPTANCE_MOVEMENT_AFTER/ { waiting = 1 }
		waiting && /setpos_exact / {
			line = $0
			sub(/^.*setpos_exact /, "", line)
			sub(/;.*/, "", line)
			print line
			exit
		}' "$ENGINE_LOG")
	if [ -z "$MOVEMENT_BEFORE" ] || [ -z "$MOVEMENT_AFTER" ] || \
		[ "$MOVEMENT_BEFORE" = "$MOVEMENT_AFTER" ]; then
		echo "acceptance scenario did not observe player movement" >&2
		exit 1
	fi
elif [ "$SCENARIO" = "ten-minute" ]; then
	for EXPECTED in RUST_TEN_MINUTE_STARTED RUST_TEN_MINUTE_COMPLETED
	do
		if ! rg -q "$EXPECTED" "$RUNTIME_LOG" "$ENGINE_LOG"; then
			echo "ten-minute scenario missing marker: $EXPECTED" >&2
			exit 1
		fi
	done
elif [ "$SCENARIO" = "physics" ]; then
	for EXPECTED in \
		RUST_PHYSICS_INTERACTION_STARTED \
		RUST_PHYSICS_WALK_BEFORE \
		RUST_PHYSICS_WALK_AFTER \
		RUST_PHYSICS_SETTLED \
		RUST_PHYSICS_PUSHED \
		RUST_PHYSICS_AFTER \
		RUST_PHYSICS_INTERACTION_COMPLETED
	do
		if ! rg -q "$EXPECTED" "$RUNTIME_LOG" "$ENGINE_LOG"; then
			echo "physics scenario missing marker: $EXPECTED" >&2
			exit 1
		fi
	done
	# Movement first: a crate that never gets touched would otherwise look
	# like a physics failure. Both samples come from a live map, so unlike
	# the acceptance scenario this really compares two in-game positions.
	# The engine writes a marker and the console output it introduces to the
	# same log without a flush in between, so the two land on one line as
	# often as on two. Every scan below therefore reads the marker's own
	# line as well: skipping it lost the sample whenever they shared one,
	# which read as a physics failure on a run that had worked.
	# Only the coordinates are compared. Printing the matched line whole and
	# trimming it back to everything after `setpos_exact` left the engine's
	# own timestamps inside the string, and a timestamp differs between two
	# samples whatever the player did, so the comparison below could never
	# fail. A run in which the player never moved at all passed this check.
	WALK_BEFORE=$(awk '
		/RUST_PHYSICS_WALK_AFTER/ { watching = 0 }
		/RUST_PHYSICS_WALK_BEFORE/ { watching = 1 }
		watching && /setpos_exact/ {
			line = $0
			sub( /^.*setpos_exact /, "", line )
			sub( /;.*/, "", line )
			print line
			exit
		}' "$ENGINE_LOG")
	WALK_AFTER=$(awk '
		/RUST_PHYSICS_WALK_AFTER/ { watching = 1 }
		watching && /setpos_exact/ {
			line = $0
			sub( /^.*setpos_exact /, "", line )
			sub( /;.*/, "", line )
			print line
			exit
		}' "$ENGINE_LOG")
	if [ -z "$WALK_BEFORE" ] || [ -z "$WALK_AFTER" ]; then
		echo "physics scenario did not report player positions" >&2
		exit 1
	fi
	if [ "$WALK_BEFORE" = "$WALK_AFTER" ]; then
		echo "physics scenario player never moved: $WALK_BEFORE" >&2
		exit 1
	fi
	# The spawned crate must be asleep before the player walks into it and
	# awake with a nonzero velocity afterwards, so the change can only come
	# from the player.
	SETTLED_STATE=$(awk '
		/RUST_PHYSICS_PUSHED/ { watching = 0 }
		/RUST_PHYSICS_SETTLED/ { watching = 1 }
		watching && /State: / { print; exit }' "$ENGINE_LOG")
	PUSHED_STATE=$(awk '
		/RUST_PHYSICS_AFTER/ { watching = 1 }
		watching && /State: / { print; exit }' "$ENGINE_LOG")
	PUSHED_VELOCITY=$(awk '
		/RUST_PHYSICS_AFTER/ { watching = 1 }
		watching && /Velocity: / { print; exit }' "$ENGINE_LOG")
	case "$SETTLED_STATE" in
		*"State: Asleep"*) ;;
		"")
			echo "physics scenario did not report the spawned crate before the push" >&2
			exit 1
			;;
		*)
			echo "physics scenario crate never settled: $SETTLED_STATE" >&2
			exit 1
			;;
	esac
	case "$PUSHED_STATE" in
		*"State: Awake"*) ;;
		*)
			echo "physics scenario crate did not wake after the push: $PUSHED_STATE" >&2
			exit 1
			;;
	esac
	if [ -z "$PUSHED_VELOCITY" ]; then
		echo "physics scenario did not report a velocity after the push" >&2
		exit 1
	fi
	# Compare numerically: the report prints signed zeroes such as "-0.00",
	# which a textual match for "0.00, 0.00, 0.00" would miss.
	if ! printf '%s\n' "$PUSHED_VELOCITY" | awk -F'Velocity: ' '
		function abs( x ) { if ( x < 0 ) return -x; return x }
		{
			split( $2, v, "," )
			if ( abs( v[1] ) + abs( v[2] ) + abs( v[3] ) > 0.01 )
				exit 0
			exit 1
		}'; then
		echo "physics scenario crate did not move after the push: $PUSHED_VELOCITY" >&2
		exit 1
	fi
	echo "physics interaction: before [$SETTLED_STATE] after [$PUSHED_STATE] [$PUSHED_VELOCITY]"
elif [ "$SCENARIO" = "hud-audio" ]; then
	for EXPECTED in \
		RUST_HUD_AUDIO_STARTED \
		RUST_HUD_AUDIO_BASELINE \
		RUST_HUD_AUDIO_ARMED \
		RUST_HUD_AUDIO_FIRED \
		RUST_HUD_AUDIO_SPRINTED \
		RUST_HUD_AUDIO_HURT \
		RUST_HUD_AUDIO_COMPLETED
	do
		if ! rg -q "$EXPECTED" "$RUNTIME_LOG" "$ENGINE_LOG"; then
			echo "hud-audio scenario missing marker: $EXPECTED" >&2
			exit 1
		fi
	done
	# The audio device has to be live, not merely initialized.
	if ! rg -q "^\[.*\]   Rate:         (11025|22050|44100|48000)" "$ENGINE_LOG"; then
		echo "hud-audio scenario reported no live audio device rate" >&2
		exit 1
	fi
	# Gameplay sounds have to reach the mixer, and firing the SMG has to be
	# among them rather than only ambient map audio. Starting a sound is not
	# enough to show for this: a started sound is still dropped from the
	# channel list if anything about the run keeps it from being heard, so
	# these read the marker the mixer emits as it takes a channel's samples.
	SOUNDS_STARTED=$(rg -c "RUST_SOUND_STARTED " "$ENGINE_LOG" || true)
	SOUNDS_MIXED=$(rg -c "RUST_SOUND_MIXED " "$ENGINE_LOG" || true)
	if [ "${SOUNDS_STARTED:-0}" -lt 2 ]; then
		echo "hud-audio scenario started too few sounds: ${SOUNDS_STARTED:-0}" >&2
		exit 1
	fi
	if [ "${SOUNDS_MIXED:-0}" -lt 2 ]; then
		echo "hud-audio scenario mixed too few of the ${SOUNDS_STARTED:-0} sounds it started: ${SOUNDS_MIXED:-0}" >&2
		exit 1
	fi
	if ! rg -q "RUST_SOUND_MIXED name=weapons/smg1/" "$ENGINE_LOG"; then
		echo "hud-audio scenario never mixed an SMG shot" >&2
		exit 1
	fi
	# The ammo element has to render a decreasing SMG clip, which only happens
	# if the HUD is reading live weapon state.
	AMMO_FIRST=$(rg -o -m1 "RUST_HUD_AMMO weapon=weapon_smg1 clip=[0-9]+" "$ENGINE_LOG" | \
		rg -o "[0-9]+$" || true)
	AMMO_LAST=$(rg -o "RUST_HUD_AMMO weapon=weapon_smg1 clip=[0-9]+" "$ENGINE_LOG" | \
		tail -1 | rg -o "[0-9]+$" || true)
	if [ -z "$AMMO_FIRST" ] || [ -z "$AMMO_LAST" ]; then
		echo "hud-audio scenario never rendered SMG ammo" >&2
		exit 1
	fi
	if [ "$AMMO_LAST" -ge "$AMMO_FIRST" ]; then
		echo "hud-audio scenario ammo did not decrease: $AMMO_FIRST -> $AMMO_LAST" >&2
		exit 1
	fi
	# Sprinting has to drain rendered suit power below full.
	if ! rg -q "RUST_HUD_SUITPOWER value=100.00" "$ENGINE_LOG"; then
		echo "hud-audio scenario never rendered full suit power" >&2
		exit 1
	fi
	if ! rg -q "RUST_HUD_SUITPOWER value=(9[0-9]|[0-8][0-9]?)\." "$ENGINE_LOG"; then
		echo "hud-audio scenario suit power never drained" >&2
		exit 1
	fi
	# Damage has to move rendered health off its starting value.
	HEALTH_VALUES=$(rg -o "RUST_HUD_HEALTH value=[0-9]+" "$ENGINE_LOG" | rg -o "[0-9]+$")
	HEALTH_FIRST=$(printf '%s\n' "$HEALTH_VALUES" | head -1)
	HEALTH_LAST=$(printf '%s\n' "$HEALTH_VALUES" | tail -1)
	if [ -z "$HEALTH_FIRST" ] || [ "$HEALTH_FIRST" = "$HEALTH_LAST" ]; then
		echo "hud-audio scenario health never changed: ${HEALTH_FIRST:-none}" >&2
		exit 1
	fi
	echo "hud/audio: $SOUNDS_MIXED of $SOUNDS_STARTED sounds mixed, SMG clip $AMMO_FIRST -> $AMMO_LAST, health $HEALTH_FIRST -> $HEALTH_LAST"
elif [ "$SCENARIO" = "soak" ]; then
	for EXPECTED in \
		RUST_SOAK_STARTED \
		RUST_SOAK_WALK_BEFORE \
		RUST_SOAK_WALK_AFTER \
		RUST_SOAK_COMPLETED
	do
		if ! rg -q "$EXPECTED" "$RUNTIME_LOG" "$ENGINE_LOG"; then
			echo "soak scenario missing marker: $EXPECTED" >&2
			exit 1
		fi
	done
	# An hour of held movement commands proves nothing if the map holds the
	# player still, so confirm the player really moves before the loop.
	# Coordinates only, for the reason given in the physics scenario above.
	SOAK_BEFORE=$(awk '
		/RUST_SOAK_WALK_AFTER/ { watching = 0 }
		/RUST_SOAK_WALK_BEFORE/ { watching = 1 }
		watching && /setpos_exact/ {
			line = $0
			sub( /^.*setpos_exact /, "", line )
			sub( /;.*/, "", line )
			print line
			exit
		}' "$ENGINE_LOG")
	SOAK_AFTER=$(awk '
		/RUST_SOAK_WALK_AFTER/ { watching = 1 }
		watching && /setpos_exact/ {
			line = $0
			sub( /^.*setpos_exact /, "", line )
			sub( /;.*/, "", line )
			print line
			exit
		}' "$ENGINE_LOG")
	if [ -z "$SOAK_BEFORE" ] || [ -z "$SOAK_AFTER" ]; then
		echo "soak scenario did not report player positions" >&2
		exit 1
	fi
	if [ "$SOAK_BEFORE" = "$SOAK_AFTER" ]; then
		echo "soak scenario player never moved: $SOAK_BEFORE" >&2
		exit 1
	fi
	# Prove the crates still exist at the end rather than having been culled
	# early, so the hour genuinely carried physics work.
	if ! rg -q "prop_physics_create: placed .* as rust_soak_crate_c" "$ENGINE_LOG"; then
		echo "soak scenario did not place its physics crates" >&2
		exit 1
	fi
elif [ "$SCENARIO" = "intro-lipsync" ]; then
	for EXPECTED in \
		RUST_INTRO_LIPSYNC_STARTED \
		'start gman : audio : gman_riseshine' \
		RUST_LIPSYNC_PHONEMES_READY \
		RUST_LIPSYNC_VISEME_ACTIVE \
		RUST_LIPSYNC_RENDER_FLEX_ACTIVE \
		RUST_LIPSYNC_CLOCK_STARTED \
		RUST_INTRO_LIPSYNC_COMPLETED
	do
		if ! rg -q "$EXPECTED" "$RUNTIME_LOG" "$ENGINE_LOG"; then
			echo "intro lip-sync scenario missing marker: $EXPECTED" >&2
			exit 1
		fi
	done
	# The clock marker above is the mixer's position in the voice channel,
	# which is what selects the current phoneme. A sentence whose clock never
	# leaves zero holds one phoneme throughout, and the face still appears to
	# move because the scene's own flexanimation tracks pose it beside the
	# phonemes, so the weights alone cannot tell the two apart.

	# Every marker above also appears for a mouth that is stuck on its first
	# frame, since each one fires once. Requiring the sampled weights to take
	# a spread of values across the sentence is what distinguishes a face that
	# animates from one that merely started.
	#
	# The two stages are read separately because they fail for different
	# reasons. The rendered weights moving says the face is driven at all.
	# The viseme magnitude moving says the speech is what drives the mouth,
	# which is the thing this gate is named for, and it is the one that
	# catches a mouth frozen on whichever phoneme happened to be current.
	for STAGE in VISEME:magnitude RENDER:total
	do
		MARKER="RUST_LIPSYNC_${STAGE%%:*}_SAMPLE"
		FIELD="${STAGE##*:}"
		DISTINCT=$(rg -o "$MARKER .*$FIELD=([0-9.]+)" -r '$1' \
			"$RUNTIME_LOG" "$ENGINE_LOG" | sort -u | grep -c .)
		if [ "$DISTINCT" -ge 5 ]; then
			continue
		fi
		echo "intro lip-sync $MARKER held only $DISTINCT distinct $FIELD values over the sentence" >&2
		if ! rg -q "RUST_VOICE_MIXER .*position=[1-9]" "$RUNTIME_LOG" "$ENGINE_LOG"; then
			# The weights look the same whether the speech drives them or
			# the scene's flex tracks do, so the mixer's own position is
			# what separates the two, and it is worth naming because a
			# stalled one has an environmental cause worth ruling out
			# first: on POSIX the mixer drops every channel while the game
			# is in the background, which silences the voice and freezes
			# its position without touching the face.
			echo "  the voice channel's mixer never advanced past sample zero, so phoneme selection could not move off the first phoneme" >&2
		fi
		exit 1
	done
fi

echo "Rust HL2 $SCENARIO passed after ${ELAPSED}s; prior logs: $PRIOR_LOGS"

}

main "$@"
