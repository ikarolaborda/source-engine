# macOS ARM64 reference baseline

Snapshot date: 2026-09-17.

## Compatibility corpus

`scripts/verify_hl2_corpus.py` discovers Steam libraries and validates the
required base-HL2 content without copying assets into the repository. On the
reference machine it found:

- the Steam-owned install (8.2 GB, 121 BSP files, 65 VPK files), App ID 220,
  `steam_legacy` branch, build `12694556`;
- a runtime staging tree (3.5 GB, 79 BSP files, 32 VPK files) containing the
  locally built ARM64 runtime;
- identical SHA-256 values in both trees for `hl2_misc_dir.vpk` and
  `d1_trainstation_01.bsp`.

The critical fingerprints observed were:

| File | SHA-256 |
| --- | --- |
| `hl2/hl2_misc_dir.vpk` | `01943ce93cb9bedecba74bd00c1823ab8d7024259283823eb4dd9fe50aa2dd14` |
| `hl2/maps/d1_trainstation_01.bsp` | `60b94d64311a51b23a7e19e21ae7cbbb4c7a6dbee0504b7e7a6116497a28b051` |

Run `scripts/verify_hl2_corpus.py --full` to emit the complete local manifest.

The Rust compatibility sweep parses all eight VPK directory files in the
install. Full payload verification succeeds for seven archives. The voice
archive contains one shipped inconsistency:
`sound/vo/novaprospekt/al_pickherup.wav` is 521,775 bytes and produces CRC
`19a18a99`, while its directory records `38d8e8b1`. The WAV reader separately
accepts an omitted physical pad byte on a final odd-sized RIFF chunk, matching
files shipped by Source, but never suppresses the CRC failure.

Of 121 installed BSP paths, 119 parse and survive a semantic repack round trip
with all 64 lump payloads and metadata unchanged. The two strict rejections are
corpus defects: `hl2/maps/d2_coast_02.bsp` is a zero-byte placeholder, and
`episodic/maps/ep1_citadel_00_demo.bsp` has an out-of-file lump range.
The same 119 maps now pass typed plane/node/leaf reference validation and full
PVS/PAS RLE validation. The Rust world service owns those structures and
supports bounded point-to-leaf and cluster visibility queries.

The bounded content audit also validates 121 BSP paths, 7,082 VMTs, 7,106 VTFs,
11,095 WAVs, 19 demos, 62 binary DMX/PCFs, three scene-image caches, 3,367 MDLs,
3,273 VVDs, 9,822 VTXs, and 2,775 PHYs. It reproduces an exact 21-entry exception manifest
at `docs/rust-port/hl2-known-content-defects.txt` and fails if a declared issue
disappears or any undeclared issue appears. Seventeen protocol-3 demos are
structurally valid; two `HLDEMO` protocol-2 developer recordings predate the
preserved `HL2DEMO` protocol-3 format. Every shipped PCF parses as DMX binary
encoding 2 / PCF format 1.

The network-message audit decodes every server-to-client packet within those
17 valid demos: 40,752 packets and 163,944 messages. This corpus fixes the
legacy network-protocol-7 widths (five-bit message IDs, four-bit string-table
IDs, 16-bit string-table update lengths, and 11-bit BSP-decal model indices),
while protocol 25 remains the strict default. Synthetic tests additionally
exercise protocol-25 server and client messages, including platform-gated
Replay and Xbox 360 fields.

The runtime staging tree contributes live save data not present in the Steam
packs. A loose-file audit validates four `JSAV` version-0x73 containers, all 48
embedded files, and 22 `VALV` map-state instances (16 embedded `.hl1` files and
six loose files). This captures saves produced by the observed ARM64 play
session and fixes the current compound-container layout in the corpus.

## Native runtime evidence

The staged launcher and all staged first-party dylibs report Mach-O ARM64.
The 2026-09-16 engine log records a 1,419-second session, three
`Host_Changelevel` events, save loads, scene activity, rendering, physics and
audio output, followed by orderly engine shutdown. This is strong evidence for
a playable native baseline, but it is not a scripted acceptance run.

The `--rust-engine` ARM64 installation now stages the Cargo-built
`source-launcher` as `hl2_launcher` instead of compiling the former C++ process
entry. Rust preserves the exact Unix argument bytes, resolves
`bin/liblauncher.dylib` relative to the executable rather than the working
directory, and invokes `LauncherMain` as one transitional callback. That
startup path creates a Rust context, routes diagnostics,
constructs validated content paths, and mounts the five base-HL2 VPKs under
the `GAME` path ID in the same priority order as `gameinfo.txt`, ahead of loose
content. This ordering matters because the reference corpus contains zero-byte
loose model placeholders whose valid versions live in the VPKs. The launcher
performs a bounded, CRC-validating read of `cfg/valve.rc` into caller-owned
storage. Ordinary relative opens across synchronized path IDs made by existing
engine consumers now return context-owned opaque Rust handles. Canonical loose
files remain streaming disk handles, while VPK entries become CRC-validated
memory cursors. Read, seek, tell, size, open-state, and close remain in Rust
behind the existing pointer-shaped `IFileSystem` adapter. Unmounted, absolute,
pure-server-tracked, and special-pack reads retain their legacy path. Positive relative existence
and filename-size queries use the same Rust registry without constructing a
file handle or copying its payload. Relative directory queries use canonical
loose paths or a binary search over each sorted VPK index.
Relative `RelativePathToFullPath`/`GetLocalPath` resolution now also comes from
the same ordered registry: Rust returns the canonical loose path or the
`<archive>.vpk/<relative>` form Source encodes for pack members, honours the
caller's `FILTER_CULLPACK`/`FILTER_CULLNONPACK` precedence, and falls back to
the native iterator for absolute paths, pure-server tracking, legacy ZIP/BSP
pack members, and buffers too small for the result. A 46-second live gate
resolved `resource/game-icon.bmp` to its canonical loose path through Rust
before server activation and shut down cleanly; the marker is now required by
`scripts/run_rust_hl2_smoke.sh`. `OpenEx` additionally defines its
resolved-filename output on every early return, and the Rust read path now
reports the same resolved name with the legacy `strdup`/`free` ownership, which
also corrected a mismatched `delete` in the engine bug reporter. ABI corpus tests
cover positive loose/VPK directories, files, and missing paths. It also loads
and structurally validates the base `scenes/scenes.image` cache through that
ordered Rust search service. Native search-path mutations now rebuild the Rust
read registry in the exact loose-directory/VPK order, including each path ID's
by-request-only visibility and trusted runtime symlink behavior. Parsed VPK
indexes remain cached across rebuilds. ZIP and embedded BSP packs remain native,
but a per-open membership guard preserves their precedence before a Rust read.
Relative wildcard enumeration now uses context-owned Rust cursors over the same
mount snapshot. It filters path IDs and by-request-only mounts, preserves mount
precedence, deterministically sorts loose entries, suppresses duplicate names,
and derives immediate virtual directories from binary-ranged VPK prefixes.
The ABI tests cover first/next/close, directory attributes, short buffers,
exhaustion, and stale handles. Absolute paths and explicit BSP-pack searches
retain their native enumerator.
The 2026-09-17 live gate synchronized three directories and five VPKs, served
`materials/console/background02.vtf` through the rebuilt registry, registered
the active BSP as one guarded legacy pack, synchronized the Rust world, and shut
down cleanly. After the cursor boundary landed, a 45-second live gate required
the opaque-read-handle marker, activated `d1_trainstation_01`, and closed
cleanly. A subsequent 43-second gate directly resolved the 174,968-byte console
background through Rust metadata before activating the same map. After all
synchronized path IDs were enabled, a 49-second gate opened the default-search
`materials/debug/debugmrmwireframe.vmt` through Rust, activated the map, and
closed cleanly. Native
loose-directory search-path mutations also
resynchronize a separate ordered Rust write registry, including each path ID's
by-request-only visibility. Rust canonically resolves every relative target used
by open-for-write, directory creation, and removal,
including Source's `GAME` to `GAME_WRITE`, `MOD` to `MOD_WRITE`, explicit
path-ID, `DEFAULT_WRITE_PATH`, and first-directory fallbacks. Absolute and
platform paths remain native. Relative write opens now return context-owned
opaque Rust file IDs; read, write, flush, seek, tell, live size, open-state, and
close operations stay behind the C ABI while existing `IFileSystem` callers
retain their pointer-shaped adapter. Rust now also performs relative recursive
directory creation and file removal after resolving the target. For rename and
writability operations, Rust searches existing loose files in native order,
filters by path ID and by-request-only visibility, replaces permissions with the
legacy owner-read/write modes, creates rename parents, and selects the rename
destination through the write aliases. Native code retains absolute/platform
mutations. The ABI tests
cover alias precedence, backslash normalization, path-escape rejection, short
buffers, registry clearing, mode validation, cursor/state operations, stale
handles, recursive directory creation, permission query/change, rename, file
removal, and lifecycle cleanup; the live gate observed a runtime-local canonical target and Rust-owned handle for
`hl2/videoconfig_mac.cfg` rather than the external asset corpus. A subsequent
46-second smoke directly exercised Rust directory creation for `downloadlists`;
the real temporary-file ABI test covers chmod, rename, and removal. The launcher drives a checked
Rust lifecycle from
launcher-ready through content-ready and legacy-running to shutdown/stopped.
A deterministic fixed-step scheduler is exposed through the same ABI and has
Rust and C++ boundary tests. The active engine also uses a context-owned Rust
frame pacer for elapsed-time accounting and every ready-versus-wait decision;
its frame-time accumulator now selects the simulation tick count, fractional
remainder, time to the next tick, paused-demo behavior, and single-player
alternate-tick debt. C++ still executes each selected legacy tick body.
When `+map` is present, the launcher also loads that BSP into the owned Rust
world, validates its tree and visibility rows, and performs a point query.
Every subsequent legacy server map spawn reloads the same Rust-owned world
through the active opaque context handle, so save reloads and level changes no
longer leave the Rust view stale. The context also owns the single pending host
operation slot, including validated bounded targets and last-request-wins
semantics matching the legacy state machine. New-game, load-game, single- and
multiplayer change-level, game-shutdown, shutdown, and restart requests cross
that slot before a C++ adapter executes the still-legacy operation body.

The Rust process entry now reaches each complete legacy engine session through
a bounded Rust-owned callback loop. Rust owns stop/restart decisions and reports the
number of completed sessions. Within each non-dedicated session, a second
Rust-owned loop controls every message/frame iteration and the stop-versus-
restart result. A post-integration 30-second live smoke completed 6,105 such
iterations, with the Rust pacer enforcing the current 3,333,333 ns minimum.
The transitional C++ callback still pumps platform messages, performs the
requested platform sleep, executes Rust-scheduled legacy ticks, integrates
Hammer, and owns module startup/teardown. The live listen server reports the
Rust tick scheduler active at its 0.015-second interval and still activates at
66.7 Hz.
The session ABI deliberately releases the host mutex before entering C++, and
the ABI smoke nests the frame loop inside the session callback to prove this
reentrant boundary cannot self-deadlock.

The host context also owns a bounded Source-style command tokenizer/queue and a
case-insensitive cvar registry with read-only enforcement and mutation
generations. Every legacy frame drains at most 64 canonical commands from that
queue into the transitional C++ executor. Startup registers and reads back a
Rust-owned diagnostic cvar and executes a queued marker through this path; the
full legacy command and cvar registries have not yet migrated.

The Rust-enabled SDL adapter also forwards physical modifier changes, mouse
buttons, and relative motion into per-context Rust state. Rust collapses
left/right modifiers to Source masks, accumulates deltas with defined overflow
behavior, and returns/clears them on consumption. SDL polling, window events,
text conversion, and final `CCocoaEvent` delivery remain transitional C++.

Live `CNetChan` instances now register opaque channel IDs in the Rust context.
Rust owns outgoing sequence advancement and the incoming duplicate,
out-of-order, packet-drop, and maximum-drop decision. Incoming handling uses a
preview/commit boundary: Rust rejects an invalid sequence before native state
changes, while an accepted sequence is committed only after the retained C++
reliable-fragment acknowledgements succeed. Rust also emits the outgoing fixed
sequence/ack/flags/reliable-state/choke/challenge header, finalizes its flags,
and generates the exact folded CRC32 used when Source enables packet checksums.
The same bounded implementation parses and validates incoming headers before
the sequence decision. Native code now owns only payload assembly,
compression/splitting, subchannels, fragment assembly, and message dispatch at
this layer. Unit and ABI tests cover the standard CRC vector, a complete
encode/finalize/parse round trip, truncation, corruption, challenge
mismatch/absence, and the existing create/reset/advance/preview/commit/remove
behavior. The 2026-09-17 live smoke registered both `loopback` and `CLIENT`
channels during server activation, observed Rust-owned outgoing header emission
and advancement plus incoming header parsing and sequence decisions on each,
and closed cleanly after one Rust-owned host session. The single-player loopback
correctly leaves checksums disabled; the C++ ABI smoke exercises Rust checksum
generation directly.

Networked string-table contents now register with the same Rust context. Rust
owns case-insensitive lookup, stable canonical indices, bounded user data,
change ticks, capacity enforcement, and HLTV/replay rollback history. The
existing C++ dictionaries remain pointer-stable mirrors for legacy consumers
and retain callbacks and bitstream encoding. Mutations enter Rust first and
the adapter disables the seam if a native mirror index or change decision ever
diverges. Unit tests cover ordinary and history-backed tables, while the C++
ABI smoke covers create/upsert/find/read/change/restore/remove across the dylib
boundary. A 2026-09-17 live `d1_trainstation_01` run registered the
`downloadables` table, reached server activation, reported no mirror mismatch,
completed one Rust-owned host session, and closed cleanly. The smoke harness
now gives cold starts at least 15 seconds after server activation before it
sends termination; the nominal 30-second request completed in 42 seconds when
map startup consumed most of the initial bound.

Server datatable metadata now crosses the same versioned ABI after the native
send-table initializer has resolved its descriptors. Rust owns validated table
and property registrations, recursive table references, the exact legacy
compatibility CRC calculation, and case-insensitive canonical server-class
IDs. Native activation is conditional on Rust reproducing the CRC already
calculated by Source, so a descriptor or ordering mismatch leaves the legacy
assignment path in control. The 2026-09-17 live `d1_trainstation_01` gate
validated 201 classes, 248 tables, and 2,230 properties at CRC `e81fa222`, then
assigned all 201 class IDs from Rust before server activation. C++ continues to
own send/receive proxy callbacks, entity packing, change detection, and
datatable bitstream emission.

Each server tick snapshot can now register one bounded, ascending array of
entity indices, network serials, and Rust-assigned class IDs. Rust owns the
immutable snapshot record, its lifetime handle, and the deduplicated pending
explicit-delete set; the native snapshot keeps the pointer-bearing mirror and
retains packed entity buffers, proxy execution, change detection, client delta
selection, and bitstream emission. Unit and ABI tests cover invalid ordering,
indices, serials, class IDs, duplicate deletes, access, removal, and lifecycle
reset. The 2026-09-17 live gate activated this path at tick 74 with 848 valid
entities, completed one Rust-owned host session, and reached `stopped` and
`>>> Engine closed` without snapshot fallback or metadata mismatch.

The installed `hl2_launcher` is now byte-identical to Cargo's release
`source-launcher`, reports as a 448,688-byte executable ARM64 Mach-O, and has
only `libSystem` as a static load dependency. A 60-second live gate entered
through that Rust `main`, activated `d1_trainstation_01`, registered the same
848-entity snapshot at tick 74, completed one Rust-owned host session, and shut
down cleanly. The first cold 30-second attempt reached its deadline before
server activation and forced the legacy material system to tear down while its
shader API was still incomplete. The smoke harness now allows a bounded
30-second activation grace before its existing 15-second active-server window;
a deliberately one-second nominal run exercised that grace, passed after 45
wall-clock seconds, and closed cleanly.

The complete `--rust-engine` game graph compiled and linked all 2,208 Waf
tasks on ARM64 after these integrations, followed by a clean incremental
rebuild and install into an isolated `/tmp` runtime. The new Rust bridge uses a
loader-relative dylib identity rather than depending on its build-tree path.
The graphical harness wakes and temporarily keeps the built-in display active,
because the retained ToGL adapter exposes no renderer record while that display
is asleep on the reference Mac.

That isolated runtime referenced the existing content corpus with symlinks,
while keeping writable config/save paths local. A bounded 30-second launch of
`d1_trainstation_01` exercised Rust executable-base discovery, VPK mounts and
reads, scene-cache validation, Cocoa/SDL initialization, OpenGL startup, server
activation at 66.7 Hz, continued frames, and orderly engine shutdown. The
repeatable harness is `scripts/run_rust_hl2_smoke.sh`; a subsequent 20-second
run also verified every Rust host lifecycle marker through `stopped`.

The harness's `save-demo` scenario drives frame-synchronized test commands in
the Rust-enabled engine build. A 98-second run on 2026-09-17, after moving
relative writable handles into Rust, loaded
`d1_trainstation_01`, held forward for two seconds and verified distinct exact
player positions, observed `npc_breen`, two `npc_citizen` thinkers, and seven
active `prop_physics` objects, recorded a protocol demo, created and reloaded a
save, changed to `d1_trainstation_02`, played the recorded demo, and shut down
cleanly. The second map synchronized a distinct Rust world containing 3,680
leaves and 1,072 visibility clusters. The generated demo then passed the Rust
parser with 405 packets and 1,185 network messages. The live ABI check reported
838 commands, 405 packet records, protocol 25, and 406 ticks. The generated save
passed the same ABI and loose-content checks with three embedded files, one map
state, and 4,095 tokens. Artifact existence, strict content validation, and
engine log markers are checked on every run; prior artifacts are moved aside so
stale output cannot satisfy the gate.
After all relative path IDs and streaming loose handles landed, a 106-second
acceptance run repeated the complete path. It directly queried `save` as a
Rust-owned loose directory, validated a 405-packet/838-command demo (1,198
decoded network messages), validated the three-file save and its 4,095-token
map state, synchronized both maps, played the demo, and shut down cleanly. The
directory marker is now a required part of the save/demo gate.
A subsequent 51-second live run required Rust wildcard enumeration and found
`d1_trainstation_01` through `maps/*.bsp` before server activation. The updated
save/demo gate then repeated movement, save/reload, the second-map transition,
demo playback, and clean shutdown in 112 seconds; its generated demo contained
404 packets and 1,176 decoded network messages. The common harness now requires
the wildcard marker rather than treating the new path as ABI-only evidence.
Two preceding cold-cache attempts exposed a test-harness race: the fixed
eight-second post-changelevel delay expired before the second server reached
`SV_ActivateServer`, and `playdemo` entered the retained engine while it was
still rebuilding edicts. Both crash reports ended in legacy
`GetContainingEntity`/`ED_Alloc`, with no Rust or filesystem frame. The scenario
now waits 20 seconds at that boundary; the passing run observed second-map
server activation before demo playback.
The same run proves the Rust-owned host-operation path: it consumed the initial
`d1_trainstation_01` new-game request, the acceptance save load, the multiplayer
transition request to `d1_trainstation_02`, game shutdown during demo playback,
and final process shutdown. Requests raised inside a legacy frame are drained
before its transition switch, preserving the old same-frame scheduling point.
After moving active tick accumulation into Rust, the scenario still records a
protocol-25 demo with 406 playback ticks and validates its save, reload, map
transition, demo playback, and shutdown sequence.
The strict demo check exposed a legacy `CUtlStreamBuffer` seek-flush defect that
dropped its retained final byte before the demo header rewrite. The seek path
now flushes every pending byte, with a Tier2 regression test that rewrites a
header and verifies the untouched tail; the complete legacy unit runner passes
with that test included.

The separate endurance script alternated held movement directions for 600
wall-clock seconds. Its bounded harness completed in 611 seconds on 2026-09-17,
observed both explicit duration markers, and shut down cleanly. This does not
yet prove broad HUD/audio behavior or the 60-minute soak.

The harness's `physics` scenario closes the direct physics-interaction gap. It
spawns a named crate under the crosshair, waits for the solver to put it to
sleep, and then punts it with the gravity gun; the gate requires the same object
to report `State: Asleep` beforehand and `State: Awake` with a nonzero velocity
afterwards, so the change can only come from the player's action. A 101-second
run on 2026-09-17 reported velocity `1.67, 0.59, -0.02` after the punt.

Building that scenario exposed three defects. `prop_physics_create` dereferenced
a null command client and crashed the server whenever it ran from the developer
console, a config file, or a test script, because the engine only attributes a
command to the listen-server host for client-issued commands. The new
`UTIL_GetCommandClientOrHost` resolves the local host in that case and returns
null only on a dedicated server, and the console commands that previously
assumed a client now use it; `FindPickerEntity` also tolerates a null player, and
`prop_physics_create` reports whether it placed the prop and accepts an optional
targetname, since spawning condenses every physics prop onto the shared
`prop_physics` classname. Separately, the scenario cannot run on the opening of
`d1_trainstation_01` at all: that map holds the player in a scripted train pod,
so the position never changes and no player action can reach a prop. The
physics scenario therefore changes to `d1_trainstation_02` first and asserts two
distinct in-game player positions before the interaction, which is stronger than
the acceptance scenario's movement check — that one samples its first position
before the map finishes loading.

The `intro-lipsync` scenario runs with audio enabled because Source advances
phoneme time from the audio mixer; the general unattended smoke and endurance
runs deliberately use `-nosound`. In a 30-second `d1_trainstation_01` run, the
G-Man scene loaded `gman_riseshine.wav`, recovered all 40 embedded runtime
phonemes, produced a nonzero `right_puckerer` viseme, passed nonzero facial flex
weights to `models/gman_high.mdl`, and shut down cleanly. Four diagnostic frames
taken during the same spoken line showed distinct mouth shapes. The permanent
gate checks the data/animation markers without retaining screenshots.
A subsequent cold 69-second run passed the same markers and clean shutdown with
live legacy `GAME` consumers reading through Rust; the first routed asset was
the 174,968-byte `materials/console/background02.vtf` payload. Replacing the
initial size probe's full CRC-checked payload read with a metadata/index query
reduced the repeat gate to 52 seconds while preserving one validated payload
read per successful open.
An intentionally short 15-second intro run then terminated the process while
the test script was still waiting. The engine now closes that handle during
`CEngineAPI::Shutdown`, before filesystem module removal; the interrupted run
completed one Rust host session and reached `>>> Engine closed` without the
former static-destructor crash. This covers user-initiated and harness timeout
shutdown independently from the still-pending uninterrupted hour gate.

The first attempted 60-minute soak exposed a separate legacy VPhysics failure
at 148.6 simulated seconds. A non-finite transform on
`train_platform_scanner` made every candidate edge distance NaN, leaving the
release build's poly/feature minimizer with a null edge after its debug assert
was compiled out. The IVP minimizer now returns its existing endless-loop
recovery result for null point/edge candidates instead of dereferencing them.
The repaired VPhysics target and complete 2,208-task graph build and install;
the next long run was manually quit by the operator after 495 seconds and did
not report an engine failure. That operator-terminated run is not counted as a
failed soak, but it also cannot close the gate: an uninterrupted full hour must
still be rerun and pass.

The corpus-backed C++ ABI smoke observes 18,654 entries in `hl2_misc`, reads
the 306-byte `cfg/valve.rc` payload, validates 1,837 scenes plus 7,333 sound
strings, and owns the train-station BSP's 5,353 leaves and 1,081 visibility
clusters through the same bridge used by the launcher.

The timing audit found and corrected an ARM-specific mismatch: `Plat_Rdtsc`
returns nanoseconds while `CalculateCPUFreq` previously used a physical or
hard-coded CPU frequency. POSIX ARM now uses a monotonic nanosecond counter and
an exact 1 GHz conversion frequency, covered by the tier0 fast-timer test.

The same test pass reproduced a legacy `CTSQueue` ARM64 race: stale dummy-node
links eventually caused a segmentation fault under the 32-thread stress case.
The transitional C++ oracle now uses a hardware fence and an ARM-only mutex
fallback around that x86-oriented queue algorithm. The complete legacy unit
runner subsequently passed three consecutive times across its 2/4/8/16/32
thread cases. The permanent Rust scheduler is expected to replace this
fallback, not reproduce the unsafe algorithm.

A debug AddressSanitizer build of the complete legacy unit runner passed on
2026-09-17, including the 2/4/8/16/32-thread `CTSList` and `CTSQueue` stress
cases. Apple's AddressSanitizer does not provide LeakSanitizer, so this result
uses `ASAN_OPTIONS=halt_on_error=1` and does not claim leak coverage.

## Rust-owned packet-entity and demo serialization

Rust now decides what each receiver's packet-entity delta contains. It compares
the two canonical snapshots narrowed by that receiver's visibility sets and
classifies every differing entity index as a creation, a removal, a recreate, or
a delta candidate; it rejects a visibility set that is out of order, duplicated,
or names an entity the newer snapshot does not hold. The native writer keeps the
packed-payload comparison that chooses between resending and preserving a delta
candidate, because that needs data the snapshot records do not carry. Rust also
encodes the per-entity delta header bits, matching all four cases of the native
variable-width index-gap encoding, and the 1072-byte demo file header, so a
recorded demo's header is produced by the same implementation that parses it.

Each of the three encoders keeps a native fallback that engages only on
disagreement and says so on the console. The `save-demo` gate requires all three
active markers and fails on any fallback marker, so a silent regression to the
legacy path cannot pass. The acceptance script turns `cl_localnetworkbackdoor`
off, because single-player otherwise hands entity state to the client in process
and never encodes a packet at all.

Running the corpus-backed form of the ABI gate also exposed two stale
expectations in its own content-dependent branch, which had never been
exercised: it required `cfg/valve.rc` to resolve as a loose disk file even
though the shipped corpus keeps it packed, and it passed a truncated length for
a second path. Both now assert the real contract, including that a genuinely
loose file still reports the disk mount, and both gate forms pass.

## Search paths keep the spelling the engine gave them

The first live run of the HUD and audio gate found the game completely silent.
The audio device was running and the game asked for sound constantly, but every
single request was refused: 897 attempts, zero channels. The cause was in the
Rust resolver rather than the mixer. It had been canonicalizing mount roots, so
a file's absolute path came back pointing at the symlink target rather than at
the search path the engine had configured. `CAudioSourceCache` attributes a
resolved path back to a search path by prefix, found no match for any of 5039
sound files, and returned no cache entry, which left every sound without an
audio source to mix. The same mismatch appeared as `/private/var` against
`/var` on macOS even without a symlinked content tree.

Mount roots are now recorded twice: once as the engine spelled them, which is
what every resolved path is built from, and once canonicalized, which is used
only to detect a traversal escaping the mount. The escape guarantee is
unchanged and a regression test mounts a root through a symlink and requires
disk paths, VPK paths, reads, sizes, directory tests, and wildcard searches to
all stay under the mounted spelling. After the fix the warning count went from
5039 to zero and the gate mixes real gameplay audio.

## The HUD and audio gate

The gate asserts the presentation surface through cheat-gated diagnostics
rather than screenshots: the mixer reports every sound it starts, and the
health, ammo and suit-power elements report what they render. Sound reporting
also names refused starts, because a run with no sound at all and a run whose
sounds are every one rejected look identical from a pass/fail count and need
very different investigation.

Two things surfaced while bringing it up. `ent_fire` resolved its player with
`UTIL_GetCommandClient()` while the neighbouring `ent_create` already used the
host fallback, so from a test script or the server console it silently did
nothing; it now matches. And the damage step could never have worked on the map
it used, because HL2 holds the `gordon_invulnerable` global on through the
opening chapter. That is authentic content behavior, not a defect, so the
script clears the global before asking for damage. The gate now requires an
SMG shot to reach the mixer, the ammo element to count a clip down, sprinting
to drain suit power, and damage to move health.

## Continuous integration

The Rust foundations job now checks out the frozen submodules recursively, so
its revision gate compares the trees the compatibility corpus was measured
against instead of reporting them missing. The gate also stopped pinning the
repository to one commit: the captured oracle revision must remain reachable
from the checked-out history, while every submodule stays pinned exactly.
`scripts/build-macos-arm64-rust.sh` builds, installs, and checks the complete
`--rust-engine` graph on the port's first platform target, including that the
installed `hl2_launcher` is the arm64 Cargo-built `source-launcher`. The
`build-macos-arm64-rust` job runs it on `macos-14`.

## Gate items still open

- The `.app` bundle is not code signed.
- G-Man scene speech, facial animation and direct physics interaction are
  automated; broader HUD/audio behavior is still open.
- The scripted ten-minute `d1_trainstation_01` movement run passed; the
  VPhysics repair is committed in the `ivp` submodule at
  `c3bf6e46b9e8e580b09c4da3091f67075fdb4de0` and built, but a new uninterrupted
  60-minute soak must pass.
- The existing 23-minute log is shorter than the required 60-minute soak.
- The provenance/rebase requirements in `provenance.md` remain open.
