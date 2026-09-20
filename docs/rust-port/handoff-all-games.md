# Handoff: a full Rust port presenting through Metal, for every Source game in this tree

**Latest installed checkpoint and next-session instructions:**
[handoff-2026-09-20.md](handoff-2026-09-20.md). The historical baseline below
remains useful, but does not override the latest installation status.

Written 2026-09-19 on branch `rust-port`, with the work of that day
uncommitted in the tree. It continues from [`handoff.md`](handoff.md) and
[`d3d9-metal.md`](d3d9-metal.md).

**2026-09-20 continuation:** the six outstanding targets now build, and
Episode One and Episode Two pass direct-map Metal smoke gates after replacing
the HL2-only startup mounts. See [the follow-up evidence](validation-2026-09-20.md)
for current results and outstanding checks. The dated findings below remain
the original baseline, not a description of the updated tree.
Later increments on the same date move generic LZSS/Snappy buffers and ordinary
relative ZIP/BSP archive reads into Rust; the linked validation records the
corpus/differential results and a still-unresolved ToGL failure-shutdown crash.
The subsequent lifecycle increment moves synchronous app-system startup and
rollback into Rust and verifies clean injected shader-init failure on both
renderers. The cause of intermittent empty ToGL adapter enumeration is still
unresolved; see the dated record rather than treating the original failure as
a pass.
The separate Startup/Shutdown path now also uses Rust lifecycle state, including
duplicate/reentrant cleanup guards. Its ABI tests, live injected-failure checks
on both renderers and fixed-camera Metal load/exit check pass; individual
subsystem bodies remain native. See the same dated validation record.
All eight game targets have since been rebuilt after those shared changes;
the installed ARM64 libraries and Rust interface exports pass a package audit.
The CI workflow now includes all eight build targets, without claiming runtime
coverage for the six games whose content is unavailable.
The next filesystem increment replaces the native pack filename/hash table and
directory parser in Rust builds with a Rust-owned metadata index. Native
absolute pack reads and BSP wildcard filtering still remain; see the dated
record for corpus, native-library and runtime checks.
The subsequent increments move the index's initial ZIP-range read and then
absolute/filtered pack payload reads, stored/LZMA decoding, CRC validation and
cursors into Rust. Native payload reader classes are excluded from Rust builds;
a thin file-handle adapter remains. Rust now also owns standalone ZIP length
discovery and BSP header/range extraction on that same retained descriptor.
Pack wildcard matching, implied directories and deduplication now also run in
Rust. Numbered ZIP candidate discovery and precedence now also run in Rust,
including first-gap termination and optional Xbox localized naming. Native
mount selection/trust and public file/find adapters still prevent full
pack-adapter retirement. See the dated validation for the tested limits.
All eight game targets have now been rebuilt again after these filesystem
increments. The final package audit passes for all 16 game libraries and 28
shared libraries. The native filesystem gate now also passes 200 map-replacement
cycles with retained old-map handles; this is not a campaign-transition test.
The following shared-owner increment consolidates ordinary mounts and explicit
pack reads onto one Rust archive index/descriptor. Native mount synchronization
now passes an opaque index instead of reopening a path/range. Obsolete C++
range mirrors and three unused private bridge exports are removed. This latest
increment rebuilt HL2/shared libraries and passed the all-eight static package
audit; the full source matrix above predates this increment. Native mount
registration, selection/trust and public adapters still remain.

The following increment moves read-path ID selection into a shared Rust policy
used by both ordinary Rust queries and the remaining native iterator. The BSP
selector uses retained typed archive provenance, not filename extensions, and
selects only GAME map packs. Native ordered registration, store-ID suppression,
trust and public adapters still remain. See the dated validation for the
compatibility checks and the retained BSP text adapter.
The next increment moves iterator order/position and duplicate-store tracking
into Rust-owned search plans/visit sets. Native snapshots retain legacy resource
references, while Rust returns selected indices. Mount registration, store-ID
assignment, Xbox exclusion inputs and trust remain native; this is not full
filesystem ownership or adapter retirement.
The next mount-table increment gives Rust ownership of the live ordered table,
iterator snapshot tables and native resource leases through explicit clone/drop
callbacks. The positive store-ID allocator is also Rust-owned. Native path
metadata, duplicate/repositioning decisions, map CRC identities and archive
reuse remain to be migrated; the opaque native resource callbacks are still
transitional. See the dated validation for lifecycle and matrix evidence.
All eight source targets have now been rebuilt and installed after this
increment; the final native gate and Episode Two Metal load/exit check pass.

**Manual gameplay handoff (2026-09-20):** prioritize migration implementation
and cheap, targeted checks. The user will play and report gameplay/rendering
flaws; do not routinely repeat expensive matrices, fuzz runs or automated
gameplay loops. The current isolated HL2 build supports an unrestricted
manual session, but is still a hybrid Rust/C++ engine, not the completed Rust
port. Launch from a terminal (3840×2160 requested; requires a suitable display):

```sh
cd /Users/ikarolaborda/source-engine
sh scripts/play_rust_hl2_metal.sh -w 3840 -h 2160
```

Choose New Game or Load Game. Rust integration is compiled into this launcher;
there is no additional Rust activation flag. This command has no test script,
automatic quit, muted sound or disabled mouse input. Full campaign correctness
and actual 4K presentation have not been verified. The existing automated gates
establish load/exit only, not gameplay or dialogue correctness.
The play script opens the menu without `+map`, enables full captions and raw
mouse input, and leaves video settings selectable. Omit `-w/-h` on subsequent
launches to use the saved resolution. After the user quit, the Metal video-options
correction and lightmap-lock fix were installed in `out-rust-allgames`; the
shared libraries and launcher in `out-rust-episodic` were also refreshed.

**Manual-play lightmap report:** `wood/woodfloor005a` and many other surfaces
appear black, but the floor texture appears with `mat_fullbright 1`. A small
Metal ABI reproduction confirms that writable locks were zeroing untouched
lightmap texels when the engine relocks an entire page and edits only part of
it. The Rust device now preserves native-format texture contents before a
writable lock. The old installed library fails the reproduction; the rebuilt
library passes for LDR, integer HDR and float HDR. This fixes a shared resource
bug, not a material-name special case. Gameplay confirmation and performance
impact of synchronized readback remain unverified; expanded-format upload
semantics remain unchanged. See `rust/tests/metal_texture_lock.py`.

**User clarification:** the Episode Two lips/speech report concerns the antlion
defense while the vortigaunts heal Alyx (`ep2_outland_02`). The scene is no longer
unknown. Walking/turning loops are not an acceptance test for this report; a
relevant check must capture visible speaking NPCs and their audio in that scene.
The report remains unresolved.

## The rule this is written under

Every statement below is one of three things, and says which:

- **Finding** — checked here, by a compiler, test, count or source file on
  disk (tier 1) or by running the game and reading its log or its own frames
  (tier 2). Each carries an evidence number, `E1`..`E35`, and section 8 gives
  the command behind each one.
- **Hypothesis** — something the evidence points at but does not establish. It
  is labelled and says what would settle it.
- **Not checked** — named as such, with the check that would do it.

There are no effort estimates here. Nothing measured says how long any of it
takes, so a number would be a guess. Sizes are given where they were counted.
A first draft of this document was audited by an independent reviewer against
the evidence file and the repository; what it found wrong is corrected below.

## 1. What "all Source games" means in this tree

**Finding (E5, E6).** The build scripts list eight build targets that have
both a client and a server: `hl2`, `hl2mp`, `hl1`, `hl1mp`, `episodic`,
`portal`, `cstrike`, `dod`. `--build-games` selects one per build and defaults
to `hl2` (E1). Listed is all that is known of six of them: see section 4.

**Finding (E6, E10).** `tf` appears only in the server list, and what it names
is not in the tree: `game/server/server_tf.vpc`, `server_econ_base.vpc` and
`game/server/tf/` do not exist. There is no `tf` client entry. Team Fortress 2
cannot be built from this tree.

**Finding (E3).** Game-specific source directories exist for `cstrike`, `dod`,
`episodic`, `hl1`, `hl2`, `hl2mp`, `portal`, in the sizes listed in section 2.

**Finding (E7, E8).** The one Steam library checked,
`~/Library/Application Support/Steam/steamapps/common`, holds only
`Half-Life 2`, whose folder holds `hl2`, `episodic` and `ep2`, and a
`lostcoast` that contains nothing but `bin`. A second copy of the `hl2` and
`platform` content exists under `~/Documents/Gaming/Half Life 2`. No other
location was searched.

**Finding (E13).** Episode One and Episode Two are one build target: both
`gameinfo.txt` files name `episodic/bin`.

So, of the eight targets, **two can be tested with the content found**
(`hl2`, and `episodic`, which is two titles), and **six cannot**: `hl2mp`,
`hl1`, `hl1mp`, `portal`, `cstrike`, `dod`.

## 2. How much is C++, measured

**Finding (E14, E14b).** First-party C++ tracked by git outside `thirdparty/`:
**9,031 files, 4,085,640 lines** (3,363,536 non-blank). Two counting methods
agree. `handoff.md` gives 1,572,433 lines for the same 9,031 files; that figure
could not be reproduced by either method and how it was counted is not
recorded.

**Finding (E14b).** Leaving out `dx9sdk`, `external`, `hammer`, `utils` and
`togles` leaves 3,025,673 lines in 6,961 files.

**Finding (E15).** Rust under `rust/`, tracked and untracked: 66 files, 50,399
lines, which is 1.2 percent of the two together by line.

**Finding (E16).** Where the C++ is, by top-level directory, in lines:

| | | | |
| --- | --- | --- | --- |
| `game` 1,215,740 | `utils` 503,544 | `common` 429,974 | `public` 310,471 |
| `engine` 294,205 | `dx9sdk` 224,322 | `hammer` 183,857 | `materialsystem` 155,897 |
| `vgui2` 127,674 | `external` 118,820 | `movieobjects` 41,489 | `gameui` 40,950 |
| `gcsdk` 40,244 | `tier1` 29,711 | `togles` 29,424 | `togl` 28,041 |
| `tier0` 26,075 | `vphysics` 22,965 | `tools` 22,725 | `particles` 19,206 |

**Finding (E17).** Game-specific code, client + server + shared, in lines:
`hl2` 212,792; `cstrike` 113,011; `dod` 63,845; `hl1` 54,654; `portal` 54,234;
`episodic` 24,153; `hl2mp` 22,902.

**Finding (E14, E31).** The Metal work of 2026-09-19, all of it untracked,
added 3,396 lines of C++ in 5 files (`tometal/`, `public/tometal/`,
`public/rust/source_d3d9.h`, 582 of them ToGL's D3DX arithmetic carried over)
and 8,418 lines of Rust (`rust/crates/source-d3d9` 7,826,
`rust/crates/source-abi/src/d3d9.rs` 592).

## 3. The plan's two checks, today

**Finding (E14).** Check 1, no first-party C++: 9,031 tracked files remain, and
5 more are untracked.

**Finding (E18).** Check 2, presents through Metal with ToGL gone: `togl/` and
`public/togl/` exist, and `SDL_GL_CreateContext` is called at
`appframework/sdlmgr.cpp:981` and `:1097`. The first sits inside
`if ( !m_bMetal )`. **Not checked:** whether a `-metal` run ever reaches the
second, in `CreateExtraContext`.

## 4. Metal, target by target

### `hl2` — run

**Finding (E29b).** With `EXTRA_LAUNCHER_ARGS="-metal"` the smoke,
direct-physics, HUD/audio, save/demo and intro lip-sync gates each end in
"passed"; the smoke gate without `-metal` still passes. A full-screen run
created a 3024x1900 back buffer against a 3024x1898 drawable.

**Reported, not re-run for this document (E29).** Earlier the same day: a
scripted walk through `d1_canals_03`, `d1_town_01` and `d1_canals_07` quit with
status 0 and no device error lines; a `d1_canals_07` autosave made by the ToGL
build loaded in the installed folder; `cl_showfps` read 110 frames a second on
`d1_canals_03`. The logs were in the session's scratch directory and are not in
the evidence file.

**Finding (E26).** Twenty engine screenshots 0.3 s apart through G-Man's
speech, taken by the same script with and without `-metal`. On a contact sheet
of eight pairs the mouth changes from shot to shot in both rows and each pair
looks alike. This was judged by eye; no pixel difference was computed. The
screenshots reach disk through `GetRenderTargetData` and a read-only lock, so
that path returned usable pixels on Metal in these runs.

### `episodic` — built; Episode Two run; Episode One not

**Finding (E20).** `--build-games=episodic` builds and installs beside the
`hl2` build: the script exits with status 0 and the client, server and
`libshaderapimetal` libraries are present.

**Finding (E23).** `-game ep2 -metal` with the map started from a config file
loads `ep2_outland_02`: the server activates at 31 s, the device's counter
passes frame 6,600 within 150 s, 132 back-buffer frames were dumped, and the
log holds no device error line. Sampled frames show particles, a dynamic light,
the achievement toast, characters and the vortigaunt's glow effects.

**Finding (E23).** That run created 1,315 shaders. The first word of every
file is `fffe0200`, `ffff0200` or `ffff0201`: `vs_2_0`, `ps_2_0`, `ps_2_b`.

**Finding (E27).** Twenty-four paired screenshots of `ep2_outland_01`'s
opening look alike on a contact sheet, and no speaking character is in any of
them.

**Not checked: lip movement and speech in Episode Two.** During the Episode
Two session the user reported that "the npc lips and speeches don't look
well". It has not been reproduced and the scene is not known. What was
observed is narrower: on Half-Life 2's intro the two renderers' mouths look
alike (E26). **Hypothesis:** if the fault shows on Half-Life 2 as well, it is
in code both renderers share rather than in the Metal device. The check: E26's
script on a scene where an Episode Two character speaks in view, with and
without `-metal`.

**Not checked: Episode One.** No Episode One map was run. The check:
`-game episodic -metal +exec <cfg holding "map ep1_citadel_04">` with
`SOURCE_D3D9_SHOT_DIR` set; `ep1_citadel_04` is a name a `strings` search
finds in `ep1_pak_dir.vpk`.

### `hl2mp`, `hl1`, `hl1mp`, `portal`, `cstrike`, `dod` — not checked

No content for them was found (E7, E8). `out-rust-ci` holds game binaries only
for `hl2` (E9) and `out-rust-episodic` only for `episodic` (E20). Whether these
six build with `--rust-engine` is not checked. Lost Coast is not a build target
in either table (E5, E6) and its folder holds no content (E8).

## 5. What stopped the second target tried, before the renderer was reached

**Finding (E21, E22).** `launcher/launcher.cpp` names Half-Life 2 in its Rust
start-up: `"hl2"` as the game directory (line 1450), five `hl2/*.vpk` archives
(1499-1506), a probe of `hl2/hl2_misc_dir.vpk` (1540), the scene cache from
`hl2/hl2_pak_dir.vpk` (1583), and a `+map` named on the command line loaded
through those mounts (1612). `-game ep2 +map ep2_outland_02` ended the process
in five seconds with `Rust BSP world load failed for maps/ep2_outland_02.bsp
(status 9)`, which `public/rust/source_abi.h` names `NOT_FOUND` (E32).

**Finding (E23).** The same map started from a config file after start-up
loads and plays. **Hypothesis:** that works because the request is then served
by the C++ filesystem, which reads its search paths from `gameinfo.txt` (E13).
Which owner served it was not observed.

**Finding (E28).** A different failure at the same line: `out-rust-ci`, whose
content is symbolic links, run without `SOURCE_HL2_CONTENT_ROOT`, ends on
`+map d1_trainstation_01` with status 8, `FORMAT_ERROR` (E32). With the
variable set the same command runs.

**Not checked.** Which other Rust-owned subsystems name Half-Life 2.
`status.md` lists what Rust owns; E23 shows Episode Two plays, not which owner
served each request.

## 6. Shaders

**Finding (E33).** The corpus is what the ToGL renderer created over five
maps, `d1_trainstation_01`, `d1_canals_03`, `d2_coast_03`, `d1_town_01`,
`d3_citadel_03`: 1,325 files, first words `fffe0200` x559, `ffff0200` x2,
`ffff0201` x764. **Finding (E30).** The corpus test passes: each translates and
compiles through `MTLDevice.makeLibrary`. The offline `metal` compiler is not
installed on this machine, per the translator author's report.

**Finding (E11b, E12).** A `strings` search of `hl2_misc_dir.vpk` finds 23
names ending `ps30`, 22 `vs30`, 43 `ps11`, 18 `ps14`, 37 `vs11`, 3 `vs14`,
beside 107 `ps20`, 105 `ps20b` and 71 `vs20`. The same search finds no such
names in `ep1_pak_dir.vpk` or `ep2_pak_dir.vpk`, and no game folder has a loose
`shaders` directory. The search would miss names stored in another form.

**Finding (E23, E33).** No shader-model-3 shader was created in any run
observed, on either target.

**Reported by the translator's author, confirmed only in part (E34).**
Shader-model 3, `vs_1_1`, `texldl`, `texldd`, `vPos`, `vFace`, multiple render
targets and depth output are covered only by hand-assembled token streams in
unit tests, and `ps_1_x` is refused. `translate/tests.rs` does hold tests named
`if_else_endif_on_a_bool_constant` and `rep_and_loop`; the rest of that list
was not confirmed here.

**Not checked.** Whether any target makes the engine create a shader-model-3
shader. The check: run it with `SOURCE_SHADER_DUMP_DIR` set and histogram the
first word of each file, as E23 did.

## 7. Known gaps in the Metal device

Tier 1, from the source named in each.

- **No multisampling.** `IDirect3D9::CheckDeviceMultiSampleType` in
  `tometal/dxabstract.cpp` returns success only for `D3DMULTISAMPLE_NONE`.
  `out-rust-ci/hl2/cfg/config.cfg` line 59 asks for `mat_antialias "8"`.
- **Nothing is cached across runs.** `shader_function` and `build_pipeline`
  in `rust/crates/source-d3d9/src/device.rs` compile and build on first use
  and keep the results in maps that die with the process.
- **Refusals that log once (E35):** a primitive type other than point, line,
  line strip, triangle list or triangle strip; a texture format outside
  `format.rs`; a depth surface locked, copied, or smaller than its colour
  target; a scaled copy from a mip level or cube face.
  `DrawIndexedPrimitiveUP` in `tometal/dxabstract.cpp` only breaks into the
  debugger. None of those lines appeared in the Episode Two log (E23).
- **A segfault reported by the shell when a run is ended with `SIGTERM`**
  (E24), seen twice. A scripted `quit` exited with status 0. No backtrace was
  taken.
- **Flashlight shadow maps.** `prepare_draw` builds a depth-compare shader
  variant when `D3DSAMP_SHADOWFILTER` is set over a depth texture. Whether any
  draw has taken that path is not known: it logs only on failure, and nothing
  failed.
- **`-metal` is opt-in** (`launcher/launcher.cpp:777`, the `shaderapimetal`
  selection). Without it a run uses ToGL.
- **macOS only.** `rust/crates/source-d3d9/src/lib.rs` compiles `device`,
  `mtl` and `objc` under `cfg(target_os = "macos")`; elsewhere only the
  translator and the format and state tables build.

## 8. Checks run, checks not run, and what to run next

**Run:** E1-E35. Raw output is in
`.agent/contexts/default/tasks/227078363354759168/large-responses/evidence.txt`,
outside git. The commands:

| | |
| --- | --- |
| E1, E5, E6 | `sed -n 309,314p wscript`; the `games = {` tables in `game/client/wscript`, `game/server/wscript` |
| E3, E10 | `ls -d game/client/*/ game/server/*/`; `ls game/server/server_tf.vpc game/server/server_econ_base.vpc game/server/tf` |
| E7, E8, E13 | `ls` of `~/Library/Application Support/Steam/steamapps/common` and of `Half-Life 2/`; each `gameinfo.txt` |
| E9 | `ls -d out-rust-ci/*/bin` |
| E11b, E12 | `strings -n 4 <dir.vpk> \| grep -iE '_(ps\|vs)[0-9]+b?$'`, counted by suffix, for every `*_dir.vpk` |
| E14, E14b | `git ls-files -z '*.cpp' '*.h' \| grep -zv '^thirdparty/' \| xargs -0 cat \| wc -l`, and the same through `xargs -0 wc -l` with totals summed |
| E15-E17, E31 | `find rust -name '*.rs' -not -path '*/target/*'`; the E14 list grouped by first path component; `find game/<side>/<game>`; `wc -l` of the new crate and `d3d9.rs` |
| E18 | `ls -d togl public/togl`; `grep -rn SDL_GL_CreateContext --include='*.cpp' .` |
| E20 | `BUILD_DIR=build-rust-episodic INSTALL_DIR=$PWD/out-rust-episodic bash scripts/build-macos-arm64-rust.sh --build-games=episodic` |
| E21, E23 | `./hl2_launcher -game ep2 -metal ... +map ep2_outland_02`, then with `+exec metaltest` (`map ep2_outland_02` in `ep2/cfg/metaltest.cfg`), `SOURCE_D3D9_SHOT_DIR` and `SOURCE_SHADER_DUMP_DIR` set |
| E22, E32 | `grep -n 'rustGameVpks\|VirtualPath\[\]' launcher/launcher.cpp`; `sed -n 40,48p public/rust/source_abi.h` |
| E25, E29b | `EXTRA_LAUNCHER_ARGS="-metal" bash scripts/run_rust_hl2_smoke.sh out-rust-ci <content> <seconds> <gate>` |
| E26, E27 | a test script of `Test_WaitForCheckPoint FinishedMapLoad`, `Test_Wait`, then `jpeg lipNN 92` / `Test_Wait` pairs, run with and without `-metal` |
| E30, E33 | `cargo test --workspace`; `SOURCE_SHADER_CORPUS=<dir> cargo test -p source-d3d9 --test translate_corpus`; `od -An -tx4 -N4` over the corpus |
| E34, E35 | `grep -n 'fn ' rust/crates/source-d3d9/src/translate/tests.rs`; `grep -n 'warn_once(' -A2 rust/crates/source-d3d9/src/device.rs` |

**Not run:** six of the eight targets; Episode One; any multiplayer session;
Episode Two lips; a soak with `-metal` (`handoff.md` records the 60-minute gate
on the legacy renderer; it was not run here); `--rust-engine` builds of
`hl2mp`, `hl1`, `hl1mp`, `portal`, `cstrike`, `dod`; Metal's API or shader
validation layers; Windows or Linux.

**What to run next.** The order is a suggestion, not a finding.

1. E26's script on an Episode Two scene with a speaking character, with and
   without `-metal`, to reproduce the lips report or fail to.
2. `--rust-engine` builds of the other six targets, which need no content.
3. The 60-minute soak with `EXTRA_LAUNCHER_ARGS="-metal"`.
4. Install one more game's content and repeat E23 on it.

## 9. What the findings add up to

- For the one second target tried, the renderer was not what stopped it; the
  Half-Life 2 names in the Rust start-up were (E21, E22), and starting the map
  from a config file got past them (E23).
- Eight targets are listed; two have been built and run with `--rust-engine`;
  six have not been built and have no content here (sections 1, 4).
- No run observed needed a shader-model-3 shader, and the translator's paths
  for one have not met a shipped shader (section 6).
- Team Fortress 2 is not in this tree (E10). `handoff.md` line 205 lists it as
  phase 9, "An authorized reimplementation on new services".
- The plan's first check reads 1.2 percent Rust by line (E14, E15).

## 10. Residual uncertainty

Section 4 rests on a few maps per target, run for a minute or two each, on one
machine with one GPU. "No device error line" means the device's own logging
printed none; no validation layer was on. Every frame comparison in this
document was made by eye on downscaled contact sheets. The user's lips report
stands unreproduced, which says what was tried, not whether the fault exists.
