# Rust migration continuation, 2026-09-20

Continues [the all-games handoff](handoff-all-games.md) from revision `f81f86f5`.
Changes and checks below are local and uncommitted. Builds and runs use
`out-rust-*` runtime folders; an inherited writable-link problem discovered
during validation is described below. This is progress toward the full port,
not its completion:
first-party C++, native client/server/VPhysics, and ToGL still remain.

## Build matrix

**Verified:** all six previously unbuilt targets now configure, build, install,
and pass the build script's native ARM64 launcher checks. `file` reports both
installed game libraries as ARM64 for each target.

| Target | Result | Runtime evidence |
| --- | --- | --- |
| `hl2mp` | build/install passed | No game content available |
| `hl1` | build/install passed after the trainer guard fix | No game content available |
| `hl1mp` | build/install passed | No game content available |
| `portal` | build/install passed | No game content available |
| `cstrike` | build/install passed | No game content available |
| `dod` | build/install passed | No game content available |

Command for each target:

```sh
BUILD_DIR=build-rust-allgames INSTALL_DIR="$PWD/out-rust-allgames" \
  bash scripts/build-macos-arm64-rust.sh --build-games=<target>
```

The first `hl1` build failed at `game/shared/gamemovement.cpp`: the shared jump
routine referenced `trainer_multijump`, whose declaration only exists for HL2
builds. The reference now has the same target guard; non-HL2 games retain zero
extra jumps. The repeat `hl1` build passed. No game content was needed to expose
or fix this compiler failure.

`episodic` also rebuilt and installed successfully into `out-rust-episodic`.
Together with the existing HL2 build, this covers all eight listed targets.
It does not make the missing TF client/server sources available.

## Rust startup content ownership

The launcher now passes `-game` (default `hl2`) to a Rust `gameinfo.txt` loader
instead of mounting a hardcoded HL2 archive list. Maps, `cfg/valve.rc`, and the
scene cache use that ordered GAME search path. The optional `SOURCE_CONTENT_ROOT`
fills missing content; the old `SOURCE_HL2_CONTENT_ROOT` name remains a fallback.

The loader uses the existing Rust KeyValues parser and preserves duplicate
GAME entries, archive-before-loose ordering as declared by the file, compound
path IDs, the two engine path tokens, sorted custom directories/VPKs, split-VPK
directory names, and native-style symlinked content. Failed initialization
leaves the prior context mounts intact. Corrupt present archives fail instead
of silently selecting a different game's data.

**Scope:** this is the startup GAME read registry. Native filesystem
initialization still supplies the subsequent complete read/write registry,
language and low-violence/HD policy, and command-line search overrides.
`#include`/`#base`, unknown conditionals, and wildcard parent directories are
explicitly rejected by the startup loader; complete native-policy replacement
remains work for host migration.

**Verified:** seven filesystem regression tests cover episode/base/loose
precedence, custom archives and chunk exclusion, external content fallback,
absolute mods and symlink reads, corrupt archives, missing paths, and
conditional declarations. An ABI regression test verifies direct world loading
through the selected game and retention of mounts after a failed replacement.

## Runtime checks

The reusable gate is `scripts/run_rust_game_smoke.sh`. It launches with direct
`+map`, clears both content-root overrides, uses isolated runtime settings,
checks Rust bootstrap/world and server-activation markers, captures actual
Metal back-buffer frames, and requires a scripted clean exit.

| Game / map | Result | Evidence directory |
| --- | --- | --- |
| Episode One / `ep1_citadel_04` | passed | `/tmp/source-rust-episodic-ep1_citadel_04.EoK26r` |
| Episode Two / `ep2_outland_02` | passed | `/tmp/source-rust-ep2-ep2_outland_02.iqw3yx` |
| HL2 / `d1_trainstation_01` | passed after capture interval correction | `/tmp/source-rust-hl2-d1_trainstation_01-metal.LWRKQu` |

```sh
bash scripts/run_rust_game_smoke.sh out-rust-episodic episodic ep1_citadel_04 300
bash scripts/run_rust_game_smoke.sh out-rust-episodic ep2 ep2_outland_02 300
```

Episode One's sampled frame shows the textured Citadel corridor, gravity gun,
crosshair, and health HUD. This is visual inspection of one captured frame,
not campaign coverage or a pixel-parity measurement. The first Episode One
attempt reached a clean exit but failed capture because the new harness had
not created its frame directory; the corrected repeat above passed.

**Focused A/B completed; reported defect unresolved:** a Griggs speaking-scene
comparison on `ep2_outland_02`. The existing
lip-sync diagnostics now accept a model-name filter (default still `gman`).
The new `rust/tests/ep2_lipsync.txt` exercises `turret_arena_vcd_1`, with audio
enabled and background muting disabled, collecting 40 in-engine JPEGs.
The user identified the antlion-defense / Alyx-healing encounter. The test
uses that map's own setup relay to spawn the defenders and healing scene,
then holds a fixed camera on Griggs for his two-line antlion dialogue.
This is a focused dialogue comparison, not a complete defense encounter.
The first A/B had different camera angles because desktop mouse input reached
the game. The harness now passes `-nomouse`; those initial captures are not a
controlled visual comparison. Speech markers only establish an advancing voice
clock and varying viseme/flex weights, not audible or visually correct lip-sync.

The camera-locked repeats passed the instrumented gate with 40 JPEGs each:

- Metal: `/tmp/source-rust-ep2-ep2_outland_02-metal.gDnkgJ`
- ToGL: `/tmp/source-rust-ep2-ep2_outland_02-togl.UlpEZW`

Both log the same camera position and angles before dialogue. Inspected frame
10 shows the speaker facing the camera in both; actor pose/timing still differ,
so this is not a pixel-parity test. At speech elapsed 0.213 seconds, viseme
magnitudes are 4.2573 (Metal) and 4.2570 (ToGL). Both report an initial
`right_drop_suppressor` output of 20.2759; later totals vary in both paths.
**Finding:** the oversized flex-rule output is not exclusive to Metal.
**Not established:** whether those outputs explain the user's reported lips,
whether the full defense dialogue fails, or whether sound is perceptually
synchronized. No animation fix has been made. Engine/launcher logs and selected
JPEGs are preserved under the task evidence directory's `ep2-metal/` and
`ep2-togl/` folders.

```sh
RUST_GAME_SCENARIO=ep2-lipsync RUST_GAME_RENDERER=metal \
  bash scripts/run_rust_game_smoke.sh out-rust-episodic ep2 ep2_outland_02 240
RUST_GAME_SCENARIO=ep2-lipsync RUST_GAME_RENDERER=togl \
  bash scripts/run_rust_game_smoke.sh out-rust-episodic ep2 ep2_outland_02 240
```

The separate HL2 direct-map gate reached its completion marker and exited
cleanly, but failed its capture requirement: no frame arrived at the original
600-present interval (`/tmp/source-rust-hl2-d1_trainstation_01-metal.0FWjho`).
It is **not passed**. The gate now samples every 60 presents; the existing
120-second HL2 smoke below is a separate successful run.
The corrected direct-map repeat (`LWRKQu`, table above) passed with captures,
bootstrap markers, and a clean exit. Its inspected late frame shows the G-Man
intro composite, not ordinary station gameplay; this does not establish
campaign or facial-animation correctness.

**Cancelled, not passed:** the walking/turning soak in `out-rust-ci` was stopped
after the user clarified that it did not test the reported speech problem and
was not needed. It reached its map and movement checks, but did not complete
the hour or emit `RUST_SOAK_COMPLETED`. Do not count this as endurance evidence.

## Verification

The inherited installers linked the entire `platform/` directory to the original
game folder. Validation found recent writes to Steam's
`platform/config/serverbrowser.vdf` and `platform/config/ingamedialogconfig.vdf`;
the old HL2 runtime also linked `hl2/glshaders.cfg` to the Documents game copy.
The harness therefore did not fully isolate writable configuration as intended.
`prepare_rust_runtime_writes.sh` now replaces those runtime links with private
copies, preserving original links in `/tmp/source-rust-writable-links.*`.
The installer and both gates use it, and the three existing runtimes have been
repaired. The original files were not restored, since their previous contents
were not recorded. Further runs must use the private copies.
SHA-256 checks of the two original Steam configuration files remained unchanged
across subsequent speech runs after the links were detached.

Using Rust 1.85.1 with rustup's shims first on PATH:

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo test --workspace
PATH="$HOME/.cargo/bin:$PATH" cargo clippy -p source-filesystem -p source-abi --all-targets -- -D warnings
PATH="$HOME/.cargo/bin:$PATH" bash rust/verify_abi.sh
```

**Verified:** workspace tests pass (277 tests, zero failures); targeted Clippy
passes; ABI verification passes 10,000 C++ lifecycle cycles, 128 C load/unload
cycles, and the LZSS differential (1,084 byte-identical buffers, 636 declined
by both codecs). This does not assert that optional external shader corpora
were supplied.

ABI verification initially rejected previously added renderer exports absent
from its allowlist. The gate now explicitly lists the existing presenter/UI
exports, macOS-only D3D9 exports, and the new gameinfo entry point, with exact
symbol matching. That verifies export coverage; runtime rendering is checked
separately by the game gates.

Raw build and check logs use `/tmp/source-rust-*-20260920.log`; preserved copies
are under `.agent/contexts/default/tasks/226414564093882368/large-responses/2026-09-20/`.

## Next host ownership increment: compressed buffers

The generic buffer path in `engine/common.cpp` now delegates Source-tagged
Snappy encoding, size queries, and SNAP/LZSS/plain decoding to Rust. The
allocating LZSS entry point also uses Rust, and the previous native error
fallback has been removed. Non-Rust builds keep the native implementation.
Rust builds omit the three native Snappy translation units from tier1;
the native codec remains in the tree as a non-Rust implementation and test
oracle. The new Rust encoder follows the [published raw Snappy format](https://github.com/google/snappy/blob/main/format_description.txt)
with a Source `SNAP` prefix. It does not promise identical match choices or
compressed bytes.

The decoder bounds length prefixes, literal lengths, all three copy forms,
overlapping copies, and output size. ABI capacity checks precede decode
allocation, error output is transactional, and recognized malformed tags never
fall through to raw copying. A zero-length ABI copy also no longer passes a
null pointer to Rust's nonnull-requiring copy primitive. The two native receive
sites that ignored decompression failure (UDP packets and reliable fragments)
now stop before parsing failed output.

**Verified:** 283 workspace tests passed; targeted Clippy and rustfmt passed.
The extended ABI gate cross-decodes 2,096 Snappy buffers in both directions and
agrees with native rejection of 9,039 malformed streams, alongside the existing
LZSS differential, lifecycle and unload checks. A bounded sanitizer fuzz run
completed 55,689 executions in 61 seconds without a finding; this is not a
sustained fuzz campaign. Native object inspection shows the generic codec path
references Rust bridge functions, not native Snappy or CLZSS methods.
The Snappy differential also passes with its C++ harness and native codec
compiled under AddressSanitizer (Rust's decoder is covered separately by the
sanitizer fuzz run).

The first rebuilt runtime attempt (`/tmp/source-rust-hl2-d1_trainstation_01-metal.e8IHHc`)
failed before engine startup with a misleading OS X 10.6.7 requirement. The
window code ignored the legacy display query's failure return and treated
missing renderer metadata as an OS version. Metal now skips
that legacy query; the retained ToGL branch and vsync query only consume valid,
initialized metadata. This was a startup defect exposed by the rebuild, not a
codec failure.

After that fix the Metal direct-map repeat passed
(`/tmp/source-rust-hl2-d1_trainstation_01-metal.vBtj9U`). The final tier1 archive
was then rebuilt without Snappy; `ar -t` confirms no Snappy members remain.
The final packaged Metal repeat passed with frame captures and clean exit:
`/tmp/source-rust-hl2-d1_trainstation_01-metal.ahQVB3`.
This host increment rebuilt HL2; the earlier all-target build matrix predates
the compressed-buffer and display-query changes.

The subsequent ToGL regression (`PaWUbD`) **failed**, with a null-source copy
in `CShaderDeviceMgrDx8::GetAdapterInfo` called by material configuration.
The native adapter list was empty. Initialization now rejects an empty list
instead of letting material startup index it. A later debugger observation of
the real macOS display database found one renderer and one adapter; an OS-level
probe also found the active Apple GPU. The reason for the earlier empty
enumeration is not established. The failure must not be erased by a later pass.
The guarded ToGL rebuild did pass a fresh direct-map run with a clean exit:
`/tmp/source-rust-hl2-d1_trainstation_01-togl.A2cwos`. This proves successful
startup on that repeat, not that empty enumeration can no longer occur.
The same guarded build then passed the Metal gate as well:
`/tmp/source-rust-hl2-d1_trainstation_01-metal.KYN9Ei`. All these game runs have
finished. Logs, the failed-run crash report, and debugger/display-probe evidence
are preserved in the task's dated `compression/` evidence folder.

**Not established:** live multiplayer packet/fragment parity, compression
performance against an agreed budget, or retirement of all native networking.
The single-player static map regression does not exercise those claims.
Raw logs are `/tmp/source-rust-compression-*-20260920.log`.

## ZIP/BSP read ownership increment

**Implemented:** native search-path synchronization now supplies ZIP/BSP file
ranges to Rust in their actual search order, with the same path IDs and
by-request-only visibility. Rust opens and owns those bytes, validates the ZIP
index/local headers, decompresses stored and LZMA entries, checks payload CRCs,
and serves the existing opaque read/seek/size/close handles. Metadata, path
resolution and wildcard queries share that registry. The native
`LegacyPackContainsFile` bypass is removed. Archive indexes are reused across
path IDs and rebuilds; unmounted maps are pruned rather than cached forever.
Recognized payload errors do not fall through to a lower mount or native IO;
a rejected ZIP/BSP mount stops initialization rather than silently dropping it.

ZIP validation now checks central-directory bounds/counts, local/central
agreement, names, flags, stored lengths and archive ranges. Split/encrypted,
ZIP64 and methods other than stored/14 are rejected. Current configurable
defaults limit archives to 512 MiB, entries to 256 MiB, and LZMA dictionaries to
64 MiB. The decoder is the pinned pure-Rust `lzma-rs` 0.3.0 dependency, not
Source's native SDK. The SDK is independently compiled only as a test oracle.

**Verified:** 288 workspace tests, targeted Clippy, rustfmt, shell syntax and
diff whitespace checks pass. `rust/tests/pak_differential.py`, integrated into
the ABI gate, compares 44 Python stored/LZMA entries and 44 Source-SDK LZMA
entries through the public mounted-file ABI, including size and seeking. The
SDK cases use size termination without an EOS marker; Python supplies the
EOS-bearing cases. The lifecycle/unload and LZSS/Snappy differential gates
remain passing. A seeded sanitizer fuzz run completed 61,509 executions in 61
seconds without a finding; this is bounded regression coverage, not a campaign.

The installed-corpus extension byte-compared **69,329 entries in 119 ZIPs**
from HL2, Episode One and Episode Two. It reports two non-auditable map files
separately: `hl2/maps/d2_coast_02.bsp` has a short header;
`episodic/maps/ep1_citadel_00_demo.bsp` has a non-ZIP/truncated pak range which
Rust rejects. Neither counts as a corpus pass. No original content was changed.

The HL2 ARM64 build/install passed. The initial Metal static-map run
`/tmp/source-rust-hl2-d1_trainstation_01-metal.6GXjhg` passed and logged a
Rust-owned, CRC-validated read of a map-embedded brick material. This is archive
ownership evidence, not proof of visual fidelity or of the NPC speech issue.
The final-build Metal run also passed with archive ownership markers required:
`/tmp/source-rust-hl2-d1_trainstation_01-metal.4Z4PGt`. No filesystem sync/read
rejection markers were present. Both runs used a fixed camera and cleanly exited.

**Failed ToGL regression:** the final-build attempt `togl.wUYeC6` enumerated no
graphics adapters. The prior guard detected this and returned initialization
failure, but cleanup then crashed in `CCvar::UnregisterConCommands`, called by
filesystem shutdown. Therefore the prior guard prevents the original adapter
indexing crash but **does not provide clean failure shutdown**. No map archive
was mounted in this failed run. The empty enumeration and cleanup lifetime
defect remain unresolved, and this run is not counted as passing.

**Still native:** initial archive discovery/indexing and BSP range extraction,
absolute encoded archive reads, pure-server tracking, explicit pack-filtered
reads/enumeration and the preload memory cache. No native pack implementation
has been removed yet. This increment rebuilt HL2 only; it does not refresh the
earlier all-eight build matrix or establish live episodic/multiplayer behavior.
Logs: `/tmp/source-rust-pak-*-20260920.log`; retained task evidence is in the
dated `pak/` folder.

## Synchronous app-system lifecycle increment

**Cause verified:** the failed shader-device initialization path unloaded its
shader module before disconnecting it, leaving registered console objects in
unmapped memory. Separately, `CAppSystemGroup::Run` returned on startup failure
without calling its cleanup path. Rust now owns the synchronous group's
Create/connect/PreInit/Init/Main sequence and reverse-order rollback of
completed steps, through native operation callbacks. Disconnection precedes
module unload. Completed PreInit is paired with PostShutdown even when later
initialization fails. Failed sessions now propagate an error to the launcher
instead of appearing to stop successfully.

The material system now unwinds its base registration on shader Init failure
and keeps the connected shader module alive until Disconnect unregisters it.
The first injected-failure run (`metal.wFGPqy`) then exposed a second defect:
texture-loader and texture-reader workers were started by the material module's
global constructor, but only stopped by a successful Init/Shutdown lifecycle.
Debugger evidence identifies both workers before material Init, and the reader
executing unmapped code during unload. Workers now start in texture-manager
Init, not module construction. Engine startup filesystem tracing is balanced
at Disconnect if normal OnShutdown was never reached. The initial failed run
and debugger evidence are retained, not relabelled as passes.

**Verified:** 291 workspace tests; targeted Clippy; rustfmt; shell syntax;
the ABI regression gates, including 1,088 new group startup/rollback scenarios
and reentrant nested groups. The shader failure is deterministic using
`-test_fail_shader_device_init`, exercised via `RUST_GAME_SCENARIO=init-failure`.
Metal `5H4scN` and ToGL `y3E3vF` both completed inner/outer group cleanup and
returned the required error exit 255, without SIGSEGV. These are failure-path
passes, not successful renderer initialization. Normal Metal static-map run
`metal.jYPvuG` also passed with Rust-owned pak reads, three completed nested
app-system groups and clean exit. Its pre-existing `ShutdownMixerControls`
trace warning also appears in the previous build's `metal.4Z4PGt` log; this
warning is not claimed fixed.

The normal ToGL repeat `togl.ngf9Vk` again found no adapters and therefore
**failed the startup gate**, but this time unwound both app-system groups,
logged the session error and exited 255 without crashing. This directly
re-exercises the original failure condition; it is evidence of corrected
cleanup, not a successful ToGL rendering run. All these processes finished.

**Scope remaining:** individual subsystems, their native callbacks and module
loading still need migration. The asynchronous group Startup/Shutdown API is
unchanged. Rust's synthetic failure matrix proves sequencing, not that every
native subsystem correctly unwinds a partially failed Connect/Init/PreInit.
Only the demonstrated shader-init failure was injected into the live engine.
The cause of intermittent empty ToGL adapter enumeration remains unresolved.
Logs are `/tmp/source-rust-appgroup-*-20260920.log`; retained task evidence is
under the dated `appgroup/` folder. This increment rebuilds HL2, not the full
eight-target matrix, and makes no speech-correctness claim.

## Split-phase app-system lifecycle increment

`CAppSystemGroup::Startup`/`Shutdown` now use the same Rust-owned completed-step
state as synchronous `Run`. Successful startup defers cleanup and does not call
Main; failure rolls back immediately, including shutdown of systems initialized
before a later failure. A public opaque handle retains that state between calls.
Shutdown validates its callback/data identity and original thread. Stale handles,
duplicate startup, reentrant cleanup and mixing Run with an active split group
are rejected before native callbacks. No registry lock spans a native callback.
Nested groups with distinct identities remain supported.

Native group guards make repeated Shutdown a no-op and prevent Run/Startup
reentry. CSteamApplication unwinds its outer group if child startup or base-path
setup fails. The old native sequencing helpers are excluded from Rust builds;
non-Rust builds retain the prior implementation. Native operation bodies and
module loading remain. Failed native callbacks still must undo partial work
they acquired themselves; this is not an audit of every subsystem's rollback.

**Verified:** 293 workspace tests, targeted Clippy, both rustfmt manifests,
shell syntax and the existing ABI/codec/archive gates. The public-ABI matrix now
checks 1,088 cases for each lifecycle, wrong callback/data/thread rejection,
invalid arguments, stale handles while a new lifecycle is active, and 1,000
reentrant cycles. The native ARM64 HL2 build/install passed.

With `RUST_GAME_SPLIT_STARTUP=1`, deterministic shader-init failure on Metal
`tXC8Pl` and ToGL `b7hGd3` unwound both groups and returned exit 255 without a
crash. Normal fixed-camera Metal `zK5mnD` passed map load, Rust pak-read ownership
and clean exit. The test-only launcher path explicitly calls Shutdown twice;
exactly two split startup/cleanup markers (one per outer/inner group) establish
that cleanup was not dispatched twice. The inner engine group continues to use
synchronous Run. These checks establish lifecycle behavior, not speech, visual
fidelity, gameplay endurance or successful ToGL rendering.
The normal launcher path (no split flag) also passed the fixed-camera Metal
map-load/pak-read/exit gate on the final build (`bFqsyq`). Both original Steam
platform-config hashes remain unchanged. All test/runtime processes finished.

**Scope:** only HL2 was rebuilt/run for this increment; Windows source changes
were not compiled here. The public native group's layout changed, so consumers
must be rebuilt together; the earlier all-eight matrix is not fresh validation.
Logs use `/tmp/source-rust-splitgroup-*-20260920.log` and retained task evidence
is under dated `splitgroup/`. The ordinary ToGL enumeration defect and the
Episode Two defense-scene lips/speech report remain open. No locomotion test
was run.

## All-game build refresh after shared lifecycle changes

All eight client/server targets were reconfigured, rebuilt and installed after
the shared native lifecycle layout change: `hl2mp`, `hl1`, `hl1mp`, `portal`,
`cstrike`, `dod`, `episodic`, then `hl2`. Every build/install and Cargo-launcher
identity check passed. The final configuration is HL2. This refresh supersedes
the earlier warning that the codec/pak/lifecycle increments had only rebuilt HL2.
It does not supply content for the six unavailable games or the missing TF
sources. No production sources were edited during the eight-target build loop.

The new read-only `scripts/verify_rust_allgames_install.py` audits the installed
launcher, all 16 client/server libraries and 28 shared libraries. It records
SHA-256 hashes, requires ARM64-only images, checks each game's CreateInterface
export, requires the synchronous/split lifecycle ABI exports, and checks that
the shared install provides consumers' Rust imports. All eight pairs passed.
The audit is not proof of source freshness by itself; the per-target build logs
supply that evidence. Seven mocked failure/success tests cover wrong launcher,
architecture, missing binaries/factories/lifecycle exports and unresolved Rust
imports. The workflow now builds/audits all eight targets; shared Metal device
tests run in the HL2 job only. The audit tests also run in the Linux foundations
job. Workflow YAML and matrix membership were checked locally; no GitHub run
was triggered or observed.

Rust cross-checks for Linux and Windows initially compiled with five warnings
in the presenter module. The Linux strict Clippy reproduction failed because
the existing CI treats warnings as errors. Mac-only imports are now cfg-gated,
and the non-Mac unsupported path explicitly consumes its validated map name.
No rendering behavior was changed. Final full-workspace/all-targets Clippy with
`-D warnings` passes on native macOS and for the Linux/Windows targets. These
are Rust compile/lint checks, not Windows/Linux engine builds or executions.
The common macOS package was rebuilt after this Rust-only fix; no game/native
layout or ABI signature changed. The final eight-game package audit passed
again. All 293 workspace tests and ABI/codec/archive gates also pass.

The isolated `out-rust-episodic` runtime was refreshed with the rebuilt shared
libraries, Rust launcher and episodic client/server pair. Episode One
`ep1_citadel_04` (`metal.2rCaXU`) passed after the native matrix refresh. Episode
Two `ep2_outland_02` (`metal.nyV4R2`) passed with the final common package and
`RUST_GAME_SPLIT_STARTUP=1`, including exactly two split startup/cleanup pairs
despite the explicit duplicate Shutdown call. Both required actual Rust pak
reads and clean exit, with a fixed camera and no movement loop. These checks
do not establish visible/audio correctness of the reported defense dialogue.
The original Steam platform-config hashes remain unchanged. All processes have
finished; no build or runtime session needs resuming.

Evidence: `/tmp/source-rust-matrix-20260920.V97B8b/`, retained under the task's
dated `matrix-refresh/` folder. The before-fix strict-lint failure is preserved.
The original full-port completion requirements remain unmet: native module
bodies, content tools and ToGL are still present.

## Rust-owned pack directory/index increment

Rust now owns the pack index used by `CZipPackFile`: central-directory
and local-header validation, canonical full-name lookup, stable entry indices,
payload offsets/sizes/method metadata and index lifetime. The C++ hash table and
directory parser are excluded from Rust builds, not kept as fallback. The Rust
index retains metadata only, with no native pointers or archive payload copy;
the existing read registry still owns its separate cached archive payloads.
The adapter's temporary read buffer is released after index creation. Local
extra fields, not central-header extra lengths, determine payload offsets.

Opaque handles support concurrent queries and reject stale handles. The ABI
returns canonical names in caller-owned buffers and reports required size
without writing short buffers. Archives are bounded to 512 MiB in both adapter
and Rust. The optional native preload cache is bypassed, serving bytes from the
archive payload instead; its reserved `__preload_section.pre` entry is hidden
from this index. This does not add support for preload-only console formats.
Directory indexing does not claim to validate payload CRCs; ordinary Rust
reads perform that validation, while retained native readers keep their prior
decoding behavior.

**Integration defect found and fixed:** `AddPackFileFromPath` did not initialize
its archive name/path-ID metadata correctly. The real-filesystem test then
crashed when GetFileTime traversed the partially initialized search path.
The path/name/ID/store registration is now complete, and the timestamp comes
from the already successful stat instead of re-entering path iteration. The
failed test log and `pak_native_read-2026-09-20-114014.ips` are retained.

**Verified:** 295 workspace tests; full-workspace/all-targets strict Clippy on
macOS and Linux/Windows Rust targets; both rustfmt checks; shell syntax; existing
ABI/codec tests. The metadata ABI differential compares names, offsets, sizes,
methods and CRC fields against Python, with distinct local/central extras,
preload exclusion, 10,000 index lifecycles, concurrent queries, invalid arguments
and short/stale-handle cases. The installed corpus matches 69,329 entries across
119 ZIPs for metadata and decoded bytes; two non-auditable maps remain excluded.
Seeded sanitizer fuzzing of both index and payload parser completed 2,665,472
executions in 61 seconds without a finding.

`rust/verify_native_pak.sh` loads the actual installed filesystem library and
compares its native absolute read/seek path against the Rust reader for 134
files: stored ZIP, Python LZMA/EOS, Source SDK LZMA/no-EOS, local-extra mismatch,
optional preload cache, and stored/LZMA embedded BSP ranges. BSP-only wildcard
enumeration also matches the index's expected names. The filesystem module
shuts down and unloads cleanly. This gate is added to the HL2 CI matrix entry;
the workflow was parsed locally, not executed remotely. The non-Rust packfile,
basefilesystem and filesystem_stdio translation units also compile; this is
not a full non-Rust runtime test.

HL2/common ARM64 build/install passed. Fixed-camera Metal `7G3Qc3` passed map
load, actual Rust index and pak-read markers, and clean shutdown. It indexed
461 entries in `d1_trainstation_01.bsp`. No locomotion or speech test was run.
Original Steam platform-config hashes remain unchanged. The eight-game package
audit passes, but only HL2/common were rebuilt in this increment; the prior
all-eight source-build matrix predates it. No public IFileSystem layout changed.

**Still native:** physical mount discovery, BSP range extraction, initial raw
buffer IO, absolute/pure-server/filtered payload handles and decoding, and the
explicit BSP wildcard filter. Those now use Rust index metadata; they have not
been retired. Full migration, ToGL enumeration and the defense-scene speech
report remain open. Logs: `/tmp/source-rust-pakindex-*-20260920.log`, retained
under the task's dated `pak-index/` folder.

## Rust-owned pack-index file I/O increment

`CZipPackFile::Prepare` now passes the archive path and ZIP byte range to
`source_pak_index_open_file`. Rust opens the regular file read-only, validates
the range against its length, allocates the bounded temporary buffer, reads
and parses the range, and releases the file/buffer before returning. The native
temporary `CUtlBuffer` and `ReadFromPack` call for index creation are gone.
Malformed archives do not fall back to native parsing. Native discovery and
BSP-header range extraction, absolute/filtered/pure-server payload readers and
BSP wildcard filtering remain. No native payload decoder was retired here.

The ABI keeps the 512 MiB bound, checked range overflow, zeroed failure outputs,
invalid UTF-8/NUL/path checks and separate IO/format statuses. Like the existing
byte-slice parser, an in-bounds zero-byte range is a valid empty index. The
metadata snapshot survives source deletion but does not bind later payload
reads to the same file revision. This is explicitly documented, not an atomic
archive-change consistency claim.

**Verified:** 295 workspace tests; strict all-workspace/all-targets Clippy on
macOS, Linux and Windows/MSVC Rust targets; both rustfmt checks; ABI, codec,
native-LZMA and lifecycle gates. File-based and slice-based index constructors
both match Python across 119 installed ZIPs / 69,329 entries; the same two
non-auditable maps remain excluded. New cases cover bad ranges/overflow,
oversize, malformed/truncated archives, invalid paths/outputs, empty ranges,
Unicode/space paths, 100 concurrent opens and detached metadata. Existing
10,000 metadata lifecycles and 1,000 concurrent queries still pass. The actual
installed filesystem passes all 134 absolute ZIP/BSP read/seek/enumeration
checks and clean unload with the new Rust index-I/O call.

The first new test incorrectly expected an empty range to fail; the existing
parser contract says it represents an empty BSP lump, so the test and API docs
were corrected. An initial Windows/GNU lint invocation used an uninstalled
target; the installed Windows/MSVC target passes. An initial package-audit
invocation named a nonexistent launcher path; the corrected package audit
passes all eight client/server pairs and 28 shared libraries, and `cmp` matches
the installed launcher to `build-rust-allgames/cargo-target/release/source-launcher`.
Only HL2/shared were rebuilt, not all eight source targets. Logs are
`/tmp/source-rust-pakindexio-*-20260920.log`.

Fixed-camera Metal `NXlIex` passes map load, the Rust index/pak-read markers and
clean shutdown. No walking/turning or speech test was run. The two original
Steam platform-config hashes remain unchanged. Evidence is retained under the
task's dated `pak-index-io/` folder, including the initial failed invocations.
The full migration and the defense-scene speech report remain open.

## Rust-owned pack payload I/O, decoding and cursors

`source_pak_index_open_file` now retains the read-only descriptor it used to
parse the index, while still releasing the temporary archive buffer. Payload
opens seek/read that descriptor under a per-archive mutex, then decode and
validate length/CRC outside the registry/context/archive locks. Replacing the
path does not substitute a new file. In-place writes are not an atomic snapshot:
changed CRC and truncated data fail before a file handle is published.
The slice-created index remains metadata-only and rejects payload opens.

`source_context_file_open_pak` returns independent, context-owned read-only
bytes/cursor through the existing file ABI. The shared `Entry::decode_payload`
performs stored/LZMA validation for both this route and ordinary relative reads.
The bounds remain 512 MiB/archive, 256 MiB/decoded entry, 64 MiB/dictionary.
Destroying the index or unmounting a pack does not invalidate an opened file.
Seeks clamp to [0,size], consistently with the prior LZMA reader; this fixes
the prior stored reader's unsigned negative-seek wrap to EOF. Invalid origins
leave the cursor unchanged. Native `FT_PACK_TEXT` CRLF handling is preserved.

`CZipPackFileHandle`, `CLZMAZipPackFileHandle`, and `CZipPackFile::ReadFromPack`
are excluded from Rust builds. `CRustPackFileHandle` forwards file operations
without owning bytes, a cursor or decoder. Successful index preparation closes
the native discovery handle, and BeginMapAccess no longer reopens one. Symbol
inspection confirms only the Rust adapter remains; the native-filesystem gate
now enforces that retirement. This does not remove all first-party C++ or the
remaining mount discovery, BSP-header range extraction and wildcard filter.

**Verified:** 297 workspace tests; strict whole-workspace/all-targets Clippy on
macOS/Linux/Windows-MSVC Rust targets; both format checks; ABI/codec/lifecycle
tests. New payload cases cover 1,000 concurrent opens, invalid/stale/null and
metadata-only handles, path replacement, CRC and truncated IO rejection, and
files surviving index destruction. Both payload routes match Python for 69,329
entries across 119 installed ZIPs; the same two non-auditable maps are excluded.
Seeded sanitizer fuzzing completed 386,200 executions in 61 seconds with no
finding (`/tmp/source-rust-pakpayload-fuzz.uonumS`).

The actual installed filesystem passes 135 file cases plus two corrupt-archive
rejections: absolute and BSP-filtered stored/Python-EOS/SDK-no-EOS LZMA reads,
deep/backward seeks, short destination buffers, CRLF text, reads after unmount,
BSP enumeration and clean module unload. Its native decoder-symbol exclusion
check passes. The three non-Rust filesystem translation units compile too;
this is not a complete non-Rust build/runtime check.

HL2/shared build/install and the all-eight static package audit pass. The full
eight-target source-build matrix still predates the filesystem increments.
Fixed-camera Metal `dTMLjt` passes map load/index, ordinary Rust pak reads and
clean exit. That game run did **not** emit the new absolute-payload marker;
the native harness is the direct evidence for that path, not the game gate.
No walking/turning or speech test ran. Original platform-config hashes match.

**Remaining limitations:** full decoded payload materialization on open may use
more memory than the old streaming decoder; campaign-scale latency/memory are
unmeasured. Live pure-server policy has not been tested; its selected pack opens
now reach the same Rust reader, but policy/precedence remain native. Logs are
`/tmp/source-rust-pakpayload-*-20260920.log`, retained in the task's dated
`pak-payload/` folder. Full migration, ToGL enumeration and defense-scene speech
remain unresolved.

## Rust-owned archive discovery and BSP pack-range extraction

`source_pak_index_open_archive` now opens a regular file once and selects either
the whole standalone ZIP or the embedded BSP ZIP. Rust reads the fixed BSP
header, validates versions 19..21, signed pack offsets/lengths, header overlap,
file bounds and unsupported outer compression, then builds the index using the
same descriptor retained for payload reads. An empty BSP lump returns NOT_FOUND
without a handle; malformed headers/ranges return FORMAT_ERROR. Range, entry
count and Unix-second timestamp are returned in a checked 32-byte C ABI record.
The 512 MiB archive bound applies before allocating the ZIP buffer. This does
not load the rest of the BSP or validate unrelated map lumps. The full Rust BSP
parser now shares `Header::parse` and still validates all its lump ranges.

Rust builds no longer execute the native BSP header read, standalone ZIP
length-seek/stat path or archive mount-open handles in the three pack mounting
paths. `CZipPackFile::Prepare` uses the Rust-discovered range and timestamp;
no native header/parser fallback remains there. Numbered ZIP existence checks
and candidate ordering remain native. A rejected numbered ZIP now decrements
the head-insertion counter, keeping the next insertion position valid.
Explicit BSP wildcard filtering and search-path trust/precedence remain native.

**Verified:** 299 workspace tests; strict whole-workspace/all-targets Clippy for
macOS/Linux/Windows-MSVC Rust targets; ABI/codec/lifecycle gates; both format
checks; shell syntax. The auto-discovered constructor, explicit-range constructor
and slice index match Python metadata; both file-backed payload routes and the
ordinary read registry match 69,329 entries across 119 installed map ZIPs. The
same two non-auditable maps remain excluded. New tests cover all accepted BSP
versions, incomplete/bad headers, signed/range/outer-compression rejection,
empty BSP absence, typed discovery, null/invalid paths and an oversized sparse
ZIP rejected before allocation. Timestamp and discovered ranges match Python.

The actual filesystem passes its 135-file read/seek/CRLF/unmount/enumeration
gate plus two corrupt-payload rejects, ten malformed BSP mount rejections,
an empty map pack, and numbered ZIP mounting at head/tail with a malformed
middle archive and a missing-number stop. The gate also checks that the built
filesystem imports auto-discovery rather than the explicit-range constructor,
and continues to enforce native payload-reader retirement. All three non-Rust
filesystem translation units compile; no full non-Rust runtime claim is made.
Seeded BSP/header sanitizer fuzzing completed 8,698,809 executions / 61 seconds
without a finding.

HL2/shared build/install and all-eight static package audit pass. Full all-eight
source-build coverage still predates these filesystem increments. Fixed-camera
Metal `nnhBVb` passes the newly required Rust range-discovery marker, index and
ordinary pak-read markers, map load and clean exit. No walking/turning or speech
test ran. Logs: `/tmp/source-rust-pakdiscover-*-20260920.log`, retained under the
task's dated `pak-discovery/` folder. The broader full-port requirements remain
open; this is archive mount ownership progress, not pack-adapter retirement.

## Rust-owned pack wildcard filtering and query cursors

`source_context_find_first_pak` now matches the selected Rust pack index and
snapshots results into a context-owned find cursor. It shares the ordinary
filesystem's ASCII-case-insensitive bytewise glob matcher (`*`, `?`, special
`*.*`), returning sorted files then sorted implied directories, deduplicating
names with file-over-directory precedence. Canonical relative paths are
returned; archive-local dot/slash normalization is supported, while root escape,
absolute paths, wildcard directory components and empty basenames are rejected.
Short output reports required size without creating/advancing a cursor. Query
cursors survive index destruction and close through the existing find ABI.

The C++ pack matcher is excluded from Rust builds. Its adapter only drains the
Rust cursor into the existing native FindData lists; public IFileSystem list
iteration, mount selection/trust and numbered ZIP candidate discovery still
remain native. This is not a claim of full public-find cursor retirement.
Basenames too long for the native MAX_PATH field are omitted, not truncated;
the Rust ABI exposes the full validated name up to the existing 1024-byte bound.

**Intentional consistency fixes:** unlike the old pack-only matcher, root files,
partial globs and multi-dot names now work according to the existing Rust find
policy. Implied directories are deduplicated before crossing the ABI, and
ordinary relative ZIP finds now handle uppercase directory prefixes and hide
the reserved preload cache. The native directory-result cleanup used scalar
`delete` for `new[]` names (also used by VPK); it now uses `delete[]`.
These are explicit behavior corrections, not a claim of byte-for-byte parity
with the old pack-specific matcher.

**Verified:** 301 workspace tests; strict whole-workspace/all-targets Clippy on
macOS/Linux/Windows-MSVC; ABI/codec/lifecycle gates; both format checks and shell
syntax. The independent Python glob oracle checks the synthetic edge cases,
a seeded pattern matrix and five patterns over each installed archive index.
The 119-map / 69,329-entry metadata and payload corpus still passes; the same
two non-auditable maps remain excluded. ABI cases cover root/multi-dot/no-extension
and Unicode literal names, duplicate/colliding directories, 300-byte names,
normalization/rejection, null/stale handles, short buffers without advancement,
index destruction and 1,000 concurrent cursors. An initial unit-test compile
failure passed a Path where the fixture helper requires a name string; corrected
without changing production behavior, with the failed log retained.

The actual loaded filesystem passes the 135-file pack gate plus new wildcard
root/partial/directory/multi-dot cases and 1,000 full plus early-close cycles.
Its import check requires the Rust pack-query entry point. All three non-Rust
filesystem translation units still compile. The pack fuzz target now includes
index wildcard queries and completed 366,820 sanitizer executions / 61 seconds
without a finding (`/tmp/source-rust-pakfind-fuzz.F2nzil`).

HL2/shared build/install and the all-eight static package audit pass; the full
all-eight source matrix remains older than these filesystem changes.
Fixed-camera Metal `CUfBMY` passes range/index/ordinary-pak reads, map load and
clean exit. The native harness, not that game run, establishes explicit BSP
wildcard behavior. No locomotion/speech test ran. Original platform-config
hashes remain unchanged. Evidence: `/tmp/source-rust-pakfind-*-20260920.log`,
retained under the task's dated `pak-find/` folder. The full migration, ToGL
enumeration and defense-scene speech report remain open.

## Numbered ZIP candidate discovery and precedence moved into Rust

`source-filesystem::pack_mounts` now owns numbered filename generation, metadata
existence checks, first-gap termination and descending numeric precedence.
`source_context_find_pack_candidates` returns a context-owned path snapshot via
the existing find-next/close ABI. The native mount adapter collects the snapshot
before mutating mounts and walks it forward; its stat loops, filename generation
and reversal are excluded from Rust builds. Native registration/reuse, path-ID
trust and placement relative to loose directories remain, so this does not
retire the full pack adapter or public filesystem ABI.

Discovery deliberately preserves the old stat policy: any metadata failure
ends that series; directories and symlinks whose targets exist are candidates;
present malformed archives do not terminate discovery. Parsing and payload
opening still validate those candidates separately. The snapshot owns path
names, not atomic file contents. Paths are never truncated. More than 65,536
candidates per series, invalid roots/languages, or overlong paths fail without
partial mount mutation. Xbox360 base/localized naming and localized precedence
are unit/ABI-tested, not Xbox runtime-verified; native caller policy still
decides when localized audio applies.

**Verified:** 304 workspace tests; strict whole-workspace/all-targets Clippy on
macOS, Linux and Windows-MSVC; ABI/codec/lifecycle checks; format and shell syntax.
The ABI gate covers numeric ordering through zip10, first missing number,
malformed/directory/symlink candidates, Unicode/spaces, independent localized
series, snapshot survival after file removal, invalid inputs, short buffers
without cursor publication/advancement, stale handles and 1,000 early closes.
The actual filesystem passes the 135-file gate plus numbered mount tests for
head/tail precedence, loose-file ordering, preexisting mounts, malformed zip1,
missing zip3, numeric zip10 priority and cross-path-ID pack reuse/unmount.
The native import gate now requires the new discovery entry point. The 119-map,
69,329-entry installed corpus passes; the same two maps remain non-auditable.
All three non-Rust filesystem translation units compile.

HL2/shared build/install and the static all-eight package audit pass. The full
eight-target source rebuild is still older than these filesystem increments.
Fixed-camera Metal `ZRJgsh` passes map load, range/index/ordinary pack reads and
clean exit; it is not evidence of live numbered ZIP precedence (the native
harness supplies that). No walking/turning or speech test ran. Original Steam
platform config hashes remain unchanged. Initial native compilation caught
two narrowing conversions and the wrong Warning overload; fixed and rebuilt.
Two package-audit invocations used nonexistent launcher paths before the final
audit compared against the actual Waf cargo-target launcher; failed logs are
retained, not represented as passing checks.

Evidence: `/tmp/source-rust-packseries-*-20260920.log`, retained in the task's
dated `pack-series/` folder. Full migration, ToGL enumeration and the actual
Episode Two defense-scene speech report remain unresolved.

## All-game source refresh after filesystem ownership changes

All eight targets were reconfigured, built and installed against the current
filesystem implementation: `hl2mp`, `hl1`, `hl1mp`, `portal`, `cstrike`, `dod`,
`episodic`, then `hl2`. This supersedes the older-source-matrix caveats in the
filesystem entries above. Each target passed its ARM64/install/export audit
and comparison with the Cargo-built launcher; the final all-eight audit passed
for 16 game libraries and 28 shared libraries. The shared-library hashes are
identical across all eight installs. Production sources were unchanged during
the matrix; only the native test harness was extended. The final Waf
configuration remains HL2 in `build-rust-allgames`, installing to
`out-rust-allgames` with `.lock-waf-build-rust-allgames`.

The configure/build commands, run once per target with the pinned rustup tools
first on PATH, were:

```sh
WAFLOCK=.lock-waf-build-rust-allgames python3 waf configure -T release \
  --disable-warns --rust-engine --prefix="$PWD/out-rust-allgames" \
  --out=build-rust-allgames --build-games=<target>
WAFLOCK=.lock-waf-build-rust-allgames python3 waf build install
```

**New native lifetime evidence:** `pak_native_read.cpp` now runs 200 map
replacement/remount cycles (100 each for head/tail placement). The generated
LZMA BSP fixtures exercise nested BeginMapAccess/EndMapAccess, repeated
registration of the same map, automatic removal of the previous map, refreshed
GAME/BSP lookup and wildcard results, and old-map decoded handles surviving
replacement plus removal of all mounts. All assertions pass, followed by clean
filesystem shutdown/unload. The existing 135-file, numbered-candidate and
wildcard gates also pass. This is an actual loaded-filesystem test, not live
gameplay level-transition coverage. The existing CI native gate includes these
new cases without a separate workflow change. Seven package-audit unit tests,
Python/shell syntax and diff checks pass; Rust production sources were unchanged
from the preceding 304-test/strict-lint run.

The isolated `out-rust-episodic` runtime now contains the matching shared
libraries, launcher and episodic game pair; its package audit passes too.
Fixed-camera Metal gates requiring real Rust archive reads pass for HL2
`d1_trainstation_01` (`JgpQzX`), Episode One `ep1_citadel_04` (`ERqWeB`) and
Episode Two `ep2_outland_02` (`8NRSxG`). Episode Two uses split startup/shutdown
and records exactly two ready/cleanup pairs despite duplicate Shutdown.
No walking/turning or speech test ran; the defense-scene report remains open.
Original Steam platform-config hashes remain unchanged.

Read-only inspection identifies the next ownership boundary: native
`SyncRustReadPaths` still reconstructs Rust mounts from `CSearchPath` state,
passing archive path/offset/length. The ordinary Rust mount then uses a separate
`Pak` cache, while the native adapter's Rust index retains a file-backed
`ArchiveIndex`. Consolidating these archive owners before replacing native
registration/trust avoids extending the current duplicate registries. This is
an observed architecture gap, not a demonstrated runtime mismatch.

Evidence: `/tmp/source-rust-fs-matrix-20260920.ZOuiij/`, retained in the task's
dated `filesystem-matrix/` directory with build/audit/native/runtime logs and
filesystem symbol/import lists. All processes finished. No commits or remote
CI runs. Six games still lack runtime content; first-party C++, ToGL, gameplay,
tools and the other full-port requirements remain outstanding.

## Ordinary and explicit pack reads now share one Rust archive owner

`source-filesystem::pack_archive::Archive` now owns metadata, typed ZIP/BSP
discovery, timestamps, retained file descriptors and validated entry reads.
The former `source-abi` archive IO implementation moved into this reusable
module; the ABI registry owns `Arc<Archive>` handles, not a second implementation.
Ordinary mounts also use this archive type. File-backed mounts do not retain a
second full compressed-byte buffer. Metadata-only constructors and memory-pack
constructors retain their separate contracts; only file-backed indexes can be
mounted through the new index-sharing API.

Native `SyncRustReadPaths` now calls `source_context_read_path_add_pak_index`
through its bridge, passing an opaque index and visibility flags, not a path and
byte range. It neither reopens nor reparses that archive. The ordinary mount and
explicit pack adapter share the validated index and descriptor, so unrelated
mount resynchronization after pathname replacement keeps both read paths on the
original file. A fresh remount sees the replacement. In-place modifications are
not an atomic snapshot: bad CRC/length or truncation fails without a fallback
to a lower-priority file. Decoded files, mounts in multiple contexts and public
index handles have independent lifetimes; the last archive reference closes
the descriptor, while decoded file cursors remain usable.

The now-unused native length mirror/getter is removed, and the native base-offset
field/getter are excluded from Rust builds. Three unreferenced private C++
bridges for raw index creation, explicit-range opening and range mounting were
removed. Their public Rust ABI equivalents remain available for compatibility
and use the shared implementation. The native import gate requires mount-by-index
and rejects those obsolete import paths. Memory-only and ordinary pack content
queries now hide the reserved preload entry consistently with explicit indexes.
Native mount registration/reuse, path-ID selection/trust, public file/find
adapters and some metadata mirrors remain; this is not full pack retirement.

**Verified:** 307 workspace tests; strict whole-workspace/all-target Clippy on
macOS/Linux/Windows-MSVC; ABI/codec/lifecycle gates; Python/shell syntax and both
format checks. New Rust tests establish shared reference counts and final-owner
release, no secondary cache for index mounts, pathname replacement, by-request
visibility, metadata/glob/read agreement, corruption rejection without loose
fallback, and memory/metadata-only distinctions. New ABI tests cover invalid
flags/IDs/slices, stale handles, index destruction while mounted, mounts across
two contexts, 1,000 concurrent reads, retained decoded files and fresh remounts.
The installed corpus still matches 119 ZIPs and 69,329 entries; the same two
non-auditable maps remain excluded. The pak fuzz target now also exercises the
shared memory owner; 232,589 sanitizer executions over 61 seconds completed
without a finding (`/tmp/source-rust-packshare-fuzz.6uiQwZ`).

The actual native filesystem passes the 135-file gate, existing wildcard and
numbered-mount cases, 200 replacement/remount cycles, and the new no-reopen
regression: replace a mounted BSP's pathname, trigger unrelated path
resynchronization, verify GAME and BSP still return the original payload, then
verify a fresh remount returns the replacement. Final shutdown/unload is clean.
An earlier run passed its native assertions but exited 2 after this assistant
edited the still-running shell wrapper, shifting its input offset. The wrapper
now parses its full body and exit before running; two subsequent complete runs
pass, including the final native-field-trim build. The failed log is retained.

HL2/shared build/install and all-eight static package audits pass. Only the
three internal filesystem translation units include `packfile.h`; all rebuild,
and their non-Rust branches still compile. The public IFileSystem ABI is
unchanged. The full all-eight source matrix in the preceding section predates
this increment; no claim of a fresh full matrix is made. The isolated episodic
runtime is updated with the final shared libraries and passes its package audit.
Fixed-camera Metal HL2 `BqxuqU` passes before the final dead-field trim; Episode
Two `toKfBY` passes on the final trimmed binaries with required archive reads
and split startup/shutdown. No movement or speech test ran. Steam platform
configuration hashes remain unchanged. Campaign memory/latency and live
pure-server behavior remain unmeasured.

Read-only inspection found the next native selection boundary in
`CBaseFileSystem::FilterByPathID`: unqualified by-request exclusion and the
special BSP request selecting GAME map packs. The existing
`SetSearchPathIsTrustedSource` implementation unconditionally marks sources
trusted under `#if 1`; its signature/whitelist logic is disabled. That behavior
was not changed here and must not be represented as verified pure-server trust.

Evidence: `/tmp/source-rust-packshare-*-20260920.log`, retained in the task's
dated `pack-share/` directory with runtime evidence. All sessions finished;
no commits. Full migration, ToGL retirement and the actual defense-scene speech
report remain unresolved.

## Shared read-path selection and typed BSP provenance

`source-filesystem::selection::path_id_matches` now owns the read path-ID
policy for Rust open/size/resolve/directory/find queries and the remaining
native `FilterByPathID` iterator. Unspecified requests skip by-request-only
mounts; explicit IDs ignore that flag and match ASCII-case-insensitively.
The reserved BSP request selects only GAME map packs. A loose directory, VPK,
standalone ZIP, literal BSP-ID mount or map under another ID cannot qualify.
The shared archive retains its typed ZIP/BSP provenance. Filename extensions
and range offsets do not establish that provenance; legacy untyped range mounts
do not qualify, while the explicit in-memory map-pak API does.

The context-free selection ABI works before bridge activation and fails closed
on invalid flags or slices. It distinguishes an explicit empty ID from no ID;
existing context read/find APIs still interpret an empty slice as unspecified.
Native mount order, store-ID duplicate suppression, registration/reuse, trust,
and public pointer-shaped adapters are unchanged. The disabled pure-server
trust implementation noted above is neither migrated nor validated here.

Ordinary Rust BSP queries now preserve archive-local slash/dot normalization,
root-escape rejection, and pack wildcard rules. A bounded find entry point
filters overlong names before publishing the cursor, without changing the
unbounded ABI or its short-buffer/no-advance contract. The first native run
caught a compatibility mismatch: POSIX FindData can hold PATH_MAX bytes, but
the pack adapter historically accepts only MAX_PATH-sized basenames. The BSP
query now retains the narrower limit, including when an overlong name sorts
before or between valid results. The failed log is retained. Source review also
identified that routing BSP text opens through the binary-only ordinary Rust
handle would lose ReadLine's CRLF translation. Those opens keep the existing
pack text adapter, with Rust-owned payload/cursor and a new native regression.

**Verified:** 309 workspace tests; strict all-target Clippy for macOS, Linux and
Windows-MSVC; public ABI/codec/lifecycle gates; 320 independent Python policy
combinations; invalid/null/stale inputs; typed-versus-untyped provenance with
misleading filename extensions; bounded/unbounded finds and cursor survival
after index destruction/unmount. Installed corpus comparison still matches
119 ZIPs and 69,329 entries, with the same two declared non-auditable exclusions.
The sanitizer pak target now compares selected map queries against index
queries: 194,665 executions in 61 seconds without a finding, corpus
`/tmp/source-rust-pathselect-fuzz.jFdYu8`.

The final real-native filesystem gate passes 135 files, numbered/wildcard cases,
200 map replacement/remount cycles, retained descriptors after pathname
replacement, and new selection checks. The latter exercise GAME priority,
by-request flags, BSP exclusion of loose/literal/ZIP mounts, native timestamp
iteration, bounded names and CRLF text reading. Final shutdown/unload is clean.
The three filesystem translation units also compile without SOURCE_RUST_ENGINE.

HL2/shared build/install and the all-eight static package audit pass, as do all
seven package-audit unit tests. The full all-eight source matrix still predates
the shared-owner and selection increments; this is not a new full matrix or
native Windows/Linux run. The isolated episodic runtime was not refreshed in
this increment. HL2 fixed-camera Metal map-load/exit `hdOMOV` passes with required
Rust archive reads. No walking/turning or speech test ran. Steam platform
configuration hashes remain unchanged. Format, Python/shell syntax and diff
checks pass. No commits.

Evidence: `/tmp/source-rust-pathselect-*-20260920.log`, retained with native
symbols/imports and runtime evidence under the task's dated `path-selection/`
directory. Next ownership boundary: native ordered mount registration,
store-ID duplicate suppression and remaining adapter state. The full migration
and the actual Episode Two defense-dialogue report remain unresolved.

## Rust-owned search traversal and duplicate-store state

The remaining native `CSearchPathsIterator` now obtains selected source indices
from a Rust `source-filesystem::search_plan::SearchPlan`. Rust owns the ordered
selection result, traversal position and duplicate physical-store set. Type and
path-ID filters, followed by platform-exclusion inputs, precede store-ID marking:
a rejected alias cannot suppress a later eligible alias. Signed store IDs are
opaque identities, including negative map IDs. VPK counts as non-pack under the
native type-filter contract; only ZIP/BSP are `GetPackFile` paths.

The context-independent ABI borrows an array of 24-byte descriptors and keeps
only selected indices, not pointers or strings. It bounds snapshots to 65,536
entries and path IDs to 4,096 UTF-8 bytes, rejects invalid flag combinations and
filters, distinguishes absent/empty requested IDs, and returns explicit
exhaustion. Reset replays an immutable plan. The native GetFirst adapter creates
a fresh plan, freezing selection metadata for that traversal. The C++ resource
snapshot and its existing pack/VPK references remain alive until the native
iterator is destroyed; returned Rust indices select those resources. Empty and
absolute pseudo-path handling remains native.

The C++ traversal counter and visited-ID vector are excluded from Rust builds.
The native fallback find adapter also replaces its visited-store vector with a
Rust-owned visit set. Handles distinguish plans from visit sets; stale/wrong-kind
operations fail closed, calls serialize across threads, and native destructors
release ownership. There is no active-context dependency during startup or
cleanup. Native mount insertion/removal/reuse, store-ID assignment, per-file
find-name deduplication and resource adapters remain. Xbox exclusion-name lookup
still supplies a native input flag, and pure-server trust is unchanged. Neither
Xbox runtime behavior nor live pure-server behavior is validated by this work.

**Verified:** 311 workspace tests; strict all-target Clippy on macOS, Linux and
Windows-MSVC; public ABI/codec/lifecycle gates; an independent Python oracle over
2,000 randomized ordered/filter/dedup snapshots; input-array/string destruction,
replay/exhaustion, signed IDs and invalid/null/stale/wrong-kind contracts; 1,000
concurrent marks and 1,000 concurrent next calls; 10,000 visit-handle lifecycles.
The new `search_plan` sanitizer fuzz target completes 4,750,121 executions in
61 seconds without a finding, comparing against a linear independent oracle.
Its corpus is `rust/fuzz/corpus/search_plan`; CI's existing target enumeration
automatically includes it. The installed archive corpus still matches 119 ZIPs
and 69,329 entries, with the same two declared exclusions.

The full real-native gate passes 135 payload files, the existing 200 map
replacement cycles and all prior fixtures, plus 1,000 search-list alias/dedup,
pack-filter, length-query and by-request refresh cycles. A final focused native
run recompiles the final harness and checks that fixture plus 1,000 fallback
find/early-close cycles while the bridge context is deliberately deactivated.
Expected context-dependent pack-discovery rejection logs occur in that fixture;
loose fallback enumeration and context-independent Rust store visits pass.
This focused run checks zero payload-file fixtures; its scope is not substituted
for the preceding full gate. Both runs shut down/unload cleanly. Non-Rust
compilation of all three filesystem translation units also passes.

HL2/shared build/install and the all-eight static package audit pass. The full
eight-target source matrix still predates the recent shared filesystem
increments; no new full-matrix or native Windows/Linux claim is made. The
isolated episodic runtime is refreshed with the current shared libraries,
launcher and episodic game pair and passes its package audit. Fixed-camera
Episode Two Metal load/exit `2AxABp` passes with Rust archive reads and two split
startup/cleanup pairs. No walking/turning or speech test ran. Steam platform
configuration hashes remain unchanged. Seven audit unit tests and
format/Python/shell/diff checks pass; no commits.

Evidence: `/tmp/source-rust-searchplan-*-20260920.log`, retained in the dated
task `search-plan/` directory with native symbols/imports and runtime evidence.
Next substantive ownership boundary remains native ordered mount registration,
identity assignment/reuse and resource-state synchronization—not another search
predicate. The complete all-games/zero-first-party-C++/ToGL-retirement goal and
the reported Episode Two defense-dialogue defect remain open.

## Rust-owned ordered mount tables and native resource leases

`source-filesystem::mount_table` now owns the live mount order and iterator
snapshot order. The native filesystem's two CSearchPath containers use an
opaque-handle facade rather than C++ ordered vectors in Rust builds. Insert,
ordered removal, swap removal, clear and final destruction are Rust-owned.
The positive process-wide store-ID allocator is also Rust-owned, with a checked
last-positive-ID boundary instead of signed overflow/reuse. Map CRC identities
remain native.

Native CSearchPath objects still carry transitional pack/VPK references. Rust
controls their leases through explicit clone/drop callbacks. Insert transfers
ownership only on success. Snapshot callbacks copy native headers and AddRef
their existing archive references; this preserves the old header-copy semantics
instead of sharing mutable header objects. Source resources are pinned during
cloning even if a callback destroys or clears the original registry. Failed
cloning drops earlier copies and publishes no partial snapshot. All callbacks
run outside the global handle-table and individual registry locks. Borrowed
native pointers require caller serialization against mutation; callback code,
data and thread-safety contracts must remain valid until all owners close.
This is explicit transitional ownership, not retirement of native resources.

Replacing relocatable vector entries exposed an old remove/re-add loop's use
of an entry pointer after removal. It now captures the relevant path/store
values and the successor's values explicitly, retaining the vector's candidate
policy without dereferencing a released object. The first native build also
caught two FOR_EACH_VEC macros requiring a vector-specific marker; they are now
ordinary indexed loops. The failed build log is retained. The non-Rust vector
implementation remains available and its three filesystem translation units
still compile.

**Verified:** 314 workspace tests and strict all-target Clippy for macOS, Linux
and Windows-MSVC; ABI/codec/lifecycle gates; 2,000 independent table mutation,
ordered/swap-removal and snapshot cases; exact drop accounting; failed ownership
transfer; failed-clone rollback; reentrant count and source destruction;
65,536-entry rejection boundary; 10,000 table lifecycles; 1,000 concurrent store
IDs; and unit coverage for allocator exhaustion. The new sanitizer `mount_table`
fuzz target completes 3,528,759 executions in 61 seconds without a finding,
checking independent ordering and resource release counts. Its corpus is
`rust/fuzz/corpus/mount_table`; CI automatically includes it via target
enumeration. The archive corpus still matches 119 ZIPs/69,329 entries, with the
same two declared non-auditable exclusions.

The initial complete native gate passes 135 payload files, 200 map-replacement
cycles, retained archive handles and all previous fixtures, plus new head/tail
insertion, duplicate no-op, re-add movement, ordered/swap removal and resource
snapshot assertions. Native imports require the new table/allocator APIs and
the old store-ID counter symbol is absent. The final complete native gate also
passes after the allocator's pure-core extraction and matrix rebuild, including
the expanded 65,536-entry/10,000-lifecycle ABI checks. Both native runs finish
with clean shutdown/unload. The expected pack-discovery status-3 diagnostics
come from the deliberately deactivated-context fallback fixture, not a failure.

All eight targets have now been configured, built, installed and package-audited
again after the recent shared-owner, selection, traversal and mount-table
increments: hl2mp, hl1, hl1mp, portal, cstrike, dod, episodic, hl2. All 28 shared
library hashes match across those installs. The final all-eight audit passes
for 16 game libraries and 28 shared libraries; configuration is restored to HL2.
This supersedes the earlier warnings that the full source matrix predates the
shared filesystem increments. It is not runtime coverage for the six games
whose content is unavailable, nor native Windows/Linux engine verification.

The isolated episodic runtime has been refreshed and package-audited.
Fixed-camera Episode Two Metal load/exit `ThjEfu` passes with required Rust
archive reads and two split startup/cleanup pairs. No walking/turning or speech
test ran. Steam platform configuration hashes remain unchanged. Seven package
audit unit tests and format/Python/shell/diff checks pass. All sessions finished;
no commits. Runtime performance budgets and campaign-level behavior are not
established by these checks.

Evidence: `/tmp/source-rust-mounttable-*-20260920.log` and the full matrix at
`/tmp/source-rust-mounttable-matrix-20260920.Yf2hxe`, retained with native
symbols/imports and runtime evidence in the task's dated `mount-table/`
directory. Native physical-path alias
matching, duplicate registration/repositioning decisions, path-ID metadata,
map CRC identities, archive reuse and trust remain. Ordinary read mounts still
synchronize from that metadata. These are the next ownership boundaries; all
native adapters and the actual defense-scene speech report remain unfinished.

## Manual gameplay: black lightmap surfaces (installed after user quit)

The user reports many black surfaces. Their `wood/woodfloor005a` floor appears
when `mat_fullbright 1` is enabled. `CMatLightmaps::LockLightmap` locks an entire
atlas page, then updates selected tiles. The Rust Metal device previously
allocated zeroed staging bytes for every writable lock and uploaded the entire
locked rectangle on unlock, erasing unchanged tiles. This is a shared resource
bug, independent of any one material or the missing-NPC scene errors.

`rust/tests/metal_texture_lock.py` reproduces the loss using a 4x4 texture on
the actual Metal device, without launching a game. The installed library fails
the untouched-texel assertion. The patched release ABI preserves the existing
native-format texture bytes before read/write locks; LDR BGRA8, HDR RGBA16-unorm
and HDR RGBA16-float cases pass. Expanded CPU/GPU format layouts still use the
existing upload-only path. Release build, formatting and diff checks pass.
No broad suite, fuzz campaign or automated gameplay test was run.

The correctness fix uses synchronized texture readback; its gameplay performance
has not been measured, and no startup speedup is claimed. After the user quit,
the normal HL2 Waf install completed and the episodic runtime's shared libraries
and launcher were refreshed. The installed ABI passes the three-format Metal
regression. Static package audits pass for all eight game pairs/28 shared
libraries in `out-rust-allgames` and the episodic pair/28 shared libraries in
`out-rust-episodic`; both launchers match the Waf Cargo output. Format, Python,
shell syntax and diff checks pass. No game was launched. The eight-source matrix
recorded above predates these final renderer/UI fixes and was not repeated.
The lightmap regression is included in the existing macOS HL2 CI job. Install
and audit logs: `/tmp/source-rust-final-{install,package,episodic-package}-20260920.log`.
The five-minute startup report, perceived slow movement and fullbright-off
gameplay confirmation remain open. Captions and menu-first startup are enabled
by the new `scripts/play_rust_hl2_metal.sh` launcher; the script does not auto-map.

## Still outstanding

- Reproduce the actual defense dialogue and establish visual/audio correctness;
  the introductory Griggs A/B alone does not close the reported issue.
  The rebuilt HL2 startup regression passed its
  120-second Metal smoke gate. The walking/turning
  soak is excluded following the user's direction; meaningful endurance coverage
  of real gameplay remains unestablished.
- Runtime checks for the six additional targets require their game content.
- GPU validation layers, multiplayer, campaign-wide coverage, shader model 3
  against shipped shaders, MSAA, and persistent shader/pipeline caches.
- The original plan's remaining host ownership, audio/UI/material/scene
  ownership, collision/physics/gameplay, content compilers, bridge retirement,
  platform expansion, and separate TF2 implementation. Passing this build
  matrix does not satisfy those requirements or authorize deleting the native
  implementations that still perform them.
