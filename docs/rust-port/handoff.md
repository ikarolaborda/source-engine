# Rust port handoff

Written at the end of the session that ended on commit
`798018ae`, branch `rust-port`, pushed to
`git@github.com:ikarolaborda/source-engine.git`.

This document says where the port actually stands against its own
definition of done, what the last session changed, what the one open
thread is and exactly what is known about it, and what remains. It is
written to be picked up cold.

## Read this first: the presentation plan changed on 2026-09-19

Everything below the next heading was written before the change and is kept
because its account of the host work and of the plan's phases is still
right. Its account of presentation is not: "The one open thread", "What to
do next" and the phase 6 list describe a route that has been left.

That route was a renderer written from nothing behind a seam that passes a
camera position. It drew the map and its static props. Every other thing the
game draws, which is characters, view models, particles, decals, water, the
HUD, the menus and the load screen, would have had to be written again
behind it one at a time, and the HUD alone had taken a session without
becoming legible. It could not have reached a playable game in any time
worth planning for.

The game is now drawn through Metal by replacing ToGL instead of replacing
the renderer. `shaderapidx9` issues the Direct3D 9 calls it always has, and
a Rust device (`rust/crates/source-d3d9`, behind the C++ classes in
`tometal/`) turns them into Metal, translating the shipped shader bytecode
to Metal Shading Language as it goes. Because nothing above Direct3D
changed, the whole game draws, and it drew correctly the first time the
device ran: the glyph problem does not exist on this path, since text is
drawn by the same material system as everything else.

**State:** `-metal` runs the menus and the campaign, full screen, at the
display's own 3024x1898 pixels, at about 110 frames a second on an M5 Pro,
with no OpenGL context in the process. The design, what was checked and how,
the details that are easy to get wrong, and what is not done are in
[`d3d9-metal.md`](d3d9-metal.md). `-metal -noshaderapi` still selects the
older renderer, which is unchanged and no longer on the path to anything.

What this does to the plan's two checks: the second, that the game presents
through Metal with no `SDL_GL_CreateContext`, is now true of a `-metal` run,
and `togl/` can be deleted once `-metal` is the default and has had a soak.
The first, no first-party C++, is untouched: `tometal/` and its headers are
about 3,100 lines of new C++, 600 of them ToGL's D3DX arithmetic carried
over, against 6,000 of Rust, and the material system is as it was. That is the honest trade. The
route that kept the C++ count falling was not going to produce a game.

## Where this stands against the plan's definition of done

The plan in `rust_engine_port_470ce0ac.plan.md` defines done by two
checks, both mechanical. Neither is close, and the numbers below are from
the current tree rather than from memory.

| Check | Required | Current |
| --- | --- | --- |
| `git ls-files '*.cpp' '*.h'` lists nothing outside `thirdparty/` | 0 files | **9,031 files, 1,572,433 lines** |
| `togl/` and `public/togl/` do not exist | absent | **both present** |
| The engine never calls `SDL_GL_CreateContext` | no callers | **still called in `appframework/sdlmgr.cpp`** |

Rust is 54 files and 41,980 lines, which is about **2.6 percent of the
first-party tree by line**. That ratio is the honest measure of how much
of the port is done. Everything below is detail underneath that number.

The plan's ten phases stand roughly as: phases 1-4 substantially done,
phase 5 (Rust-owned host) in progress and well advanced, phase 6
(presentation) in progress and early, phases 7-10 not started.

## What is actually working

These are verified by gates that run, not by inspection:

- A native macOS ARM64 build that launches, loads `d1_trainstation_01`,
  plays the intro scene with working lip-sync, and passes smoke,
  direct-physics, HUD/audio and 60-minute soak gates on the legacy
  renderer.
- Rust owns a large part of the host: filesystem and search precedence,
  VPK reads, BSP/PVS world queries, net channel and split packets,
  string tables, datatables, snapshots and entity deltas, demos, saves,
  commands and cvars, tick scheduling, and the session lifecycle. The
  detail is in `status.md`, which is long but current.
- A Metal renderer that draws the world from shipped content: BSP
  geometry, materials and textures from the real search path, baked
  lightmaps, PVS-driven visibility, brush models placed from entities,
  and static props with ambient lighting. Under `-metal -noshaderapi`
  the smoke, direct-physics and HUD/audio gates all pass.

## The one open thread

**Getting the HUD and VGUI legible through Metal.** The task id in the
todo list is `metal-hud-glyphs`.

State: the interface reaches the screen and is in the right place, but
every glyph draws as a **solid filled block** instead of a letterform.
A captured frame showing this is the clearest artefact; regenerate one
with the recipe below.

What was established this session, in order, each by measurement rather
than reasoning:

1. The panels were laid out against a screen size that did not exist.
   `shaderapiempty` had been given a built-in default of 1024x768 (by an
   earlier fix of mine for a 0x0 window), while the engine's real mode
   was 640x480. The render context falls back to the back buffer when no
   viewport is pushed, VGUI takes that as the screen, and HL2 sizes its
   HUD in proportion to it, so every glyph landed outside the clip
   rectangle. Fixed in `0d8d77aa` by latching the size at
   `CShaderAPIEmpty::SetMode`, which is the call the material system
   actually makes — the device manager's `SetMode`, which I had hooked
   first, is never reached on this path.
2. `CVideoMode_Common` never initialised `m_nUIWidth`/`m_nUIHeight`,
   which `GetModeUIWidth` reports and the root VGUI panel is sized from.
   Fixed in the same commit.
3. The overlay uploaded its rectangles against the **drawable's** size
   when they are stated in the **surface's**; on a Retina display those
   differ and the whole interface was drawn into a corner at the ratio
   between them. Fixed in the same commit.
4. The overlay was created at the frame boundary, but the engine
   rasterises its font sheets while the level loads, long before the
   first frame. Every sheet was dropped with an invalid-handle status
   nobody checked. Fixed in `798018ae`; rectangles naming a texture with
   no pixels fell from 25,652 a second to 3,627.
5. Frame recording kept only the first twelve presents, which are
   seconds after a map loads and before any scenario has acted, so no
   captured frame could ever have shown the HUD. `SOURCE_METAL_SHOT_EVERY`
   now spreads capture across a run.

### What to do next, concretely

The glyphs render as solid blocks. There are two candidate causes and
the next session should distinguish them with **one** instrumented run
rather than by reasoning:

- **The font sheet's pixels are wrong** — most likely opaque white
  everywhere, which tinted by the vertex colour gives exactly the amber
  and white blocks seen. This would mean `DrawSetSubTextureRGBA` glyph
  updates are not landing in the sheet, or the alpha channel is being
  lost in the RGBA-to-BGRA swizzle in
  `rust/crates/source-materialsystem/src/overlay.rs`.
- **The glyph texture coordinates are degenerate** — if `s0,t0,s1,t1`
  arrive as zeros, every glyph samples one texel of the sheet and fills
  its rectangle with it, which looks identical.

The diagnostic that separates them was written and then removed before
committing (it is in the session, not the tree): a per-texture-id
histogram in `Rust_ForwardQuad` in `vguimatsurface/MatSystemSurface.cpp`
recording, once a second, each id's quad count, whether Rust holds
pixels for it, the quad's size, and its texture coordinates. Text
reaches that function through `DrawFlushText` -> `DrawQuadArray`, so the
glyph quads are visible there. If the coordinates come back non-zero and
per-character, the sheet's pixels are at fault and the place to look is
`Overlay::set_texture` / `set_sub_texture`; if they come back zero, the
fault is upstream in how the batched character verts are read.

A related and separate gap, already measured: texture ids 1, 2, 3, 4 and
9 are drawn 3,627 times a second and have no pixels in Rust. These are
**material-backed images** (VTF-backed icons, the crosshair) rather than
font sheets, so they never come through `DrawSetTextureRGBA`. They will
draw as white blocks until the overlay can resolve a material's texture
through the same content path `source-materialsystem` already uses for
the world. This is a known, bounded piece of work and is not the glyph
problem.

## What remains after that

Ordered by the plan's phases. Sizing is deliberately blunt.

**Phase 6, presentation — largest remaining block before anything else
can be deleted.**
- HUD and VGUI legible and correct (the open thread above).
- Animated studio models and NPCs drawn through Metal; only static props
  draw today.
- Particles, beams, decals, water, and the level-load screen, which is
  still the legacy renderer's.
- Props culled by the leaves the prop lump lists rather than by the
  frustum alone (`prop-pvs`).
- The map's direct lights added to the ambient cube so props match the
  world they stand in (`direct-lighting`).
- The audio mixer and device in Rust; today audio is still the legacy
  mixer, and the HUD/audio gate only asserts that sounds mix.
- Offline shader packaging.
- Gates named by the plan and not yet built: campaign screenshot
  thresholds, GPU validation, Retina/fullscreen behaviour, audio-event
  parity, and an agreed performance budget.
- Only once nothing draws through it can `togl/` and `public/togl/` be
  deleted (`delete-togl`) and `SDL_GL_CreateContext` removed.

**Phase 5 remainder, host ownership.** Absolute and pure-server file
paths, ZIP/BSP pack reads, packet compression, reliable fragments and
message dispatch, pointer-stable string-table mirrors and callbacks,
datatable proxy callbacks, entity packing and payload delta emission,
platform message pumping. Each is listed with its current owner in
`status.md`.

**Phase 7, simulation and HL2 gameplay — not started.** Collision, then
movement, weapons, server entities, AI, datamaps, client prediction and
effects. Ends with deleting the legacy client, server and VPhysics
dylibs. This is the single largest phase by volume.

**Phase 8, toolchain — not started.** VTex, Studiomdl, VBSP/VVIS/VRAD,
captions, scene-image generation, DMX conversion, shader compilation.

**Phase 9, TF2 program — not started.** An authorized reimplementation
on new services, explicitly not official TF2 compatibility.

**Phase 10, bridge retirement and platforms — not started.** Remove each
adapter once no legacy consumer remains, then Windows and Linux.

**Phase 1 provenance, unresolved and worth flagging.** The plan requires
resolving the leak-derived provenance warning in `README.md` against
`LICENSE` before any public distribution. The repository is currently
pushed to a public-capable GitHub remote. This is a legal question
rather than an engineering one and no engineering work closes it.

## How to run things

```sh
# build
bash scripts/build-macos-arm64-rust.sh

# gates; drop EXTRA_LAUNCHER_ARGS to run the legacy renderer instead
EXTRA_LAUNCHER_ARGS="-metal -noshaderapi" \
  bash scripts/run_rust_hl2_smoke.sh out-rust-ci \
  "$HOME/Library/Application Support/Steam/steamapps/common/Half-Life 2" \
  280 hud-audio        # also: smoke, physics, soak

# capture frames spread across a run rather than only the first twelve
SOURCE_METAL_SHOT_DIR=/tmp/shots SOURCE_METAL_SHOT_EVERY=180 \
  EXTRA_LAUNCHER_ARGS="-metal -noshaderapi" bash scripts/run_rust_hl2_smoke.sh ...

# rust
cd rust && cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings \
  && cargo fmt --all && cargo test --workspace     # 242 tests
```

A gate run costs about 150 seconds under `-metal -noshaderapi` and about
110 on the legacy renderer, and the engine log lands in
`out-rust-ci/engine.log`.

## An honest note on pace

The HUD work took far more iterations than it should have, and the
reason is worth recording so it is not repeated. The failure was a chain
of five independent defects — an invented screen size, uninitialised UI
dimensions, a wrong coordinate space, an overlay that did not exist yet,
and a capture window that could never show the result — each of which
fully masked the ones behind it. Every probe cost a 150-second gate run,
and several of my early probes tested a hypothesis instead of measuring
state, which produced runs that eliminated nothing. Two of the five
defects were introduced by my own earlier fixes.

What worked, and what a next session should do from the start: stop
hypothesising and dump the actual state of the whole chain in one run —
walking the panel tree and printing every ancestor's size named the
defect immediately after several runs of guessing had not. Prefer one
instrumented run that prints everything to three that each test one
idea.
