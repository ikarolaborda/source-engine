# Rust engine workspace

This workspace is the ownership boundary for the staged Rust port. It starts
with format code and a versioned C ABI that can be exercised without linking
Rust types, allocators, or unwinding into the legacy runtime.

Everything below assumes the toolchain `rust-toolchain.toml` pins. A Rust
installed outside rustup, such as Homebrew's, sits ahead of rustup's shims on
`PATH` and ignores that file, which builds the engine's Rust library with a
different compiler than CI uses and hides lints CI enforces. Waf now refuses
to configure a `--rust-engine` build on a mismatch, but plain `cargo` will not
complain, so put rustup first:

```sh
export PATH="$HOME/.cargo/bin:$PATH"
cargo --version   # must match the channel in rust-toolchain.toml
```

Run all current checks with:

```sh
cargo test --workspace
python3 scripts/verify_rust_port_revisions.py
./rust/verify_abi.sh
./rust/verify_timer.sh build-rust-check
```

The crates that carry a platform backend also have a path for platforms
without one, and that path is easy to break from a Mac because nothing here
compiles it. Checking it does not need a cross linker, only the standard
library for the target:

```sh
rustup target add x86_64-unknown-linux-gnu x86_64-pc-windows-msvc
cargo check --workspace --all-targets --target x86_64-unknown-linux-gnu
cargo check --workspace --all-targets --target x86_64-pc-windows-msvc
```

Pass the installed Half-Life 2 root to `verify_abi.sh` to additionally exercise
the C++ bridge against the shipped VPK payload and scene-image cache.

Shared native ABI/layout changes require rebuilding every game consumer. For
the eight targets with both client and server sources, build into one install
(HL2 last leaves the normal development configuration selected), then audit it:

```sh
for game in hl2mp hl1 hl1mp portal cstrike dod episodic hl2; do
  BUILD_DIR=build-rust-allgames INSTALL_DIR="$PWD/out-rust-allgames" \
    bash scripts/build-macos-arm64-rust.sh --build-games="$game" || exit 1
done
python3 scripts/verify_rust_allgames_install.py out-rust-allgames \
  --cargo-launcher build-rust-allgames/cargo-target/release/source-launcher
python3 scripts/test_verify_rust_allgames_install.py
```

Keep each build's exit status/log: the read-only package audit checks ARM64
images, hashes, game factory/lifecycle exports and provision of Rust imports,
but cannot establish that pre-existing binaries were rebuilt from current
source. It also cannot establish runtime compatibility without each game's
content. TF is excluded because its client/server source set is incomplete.
The macOS Rust CI job uses these eight targets as its build/audit matrix; the
shared Metal device tests run only in the HL2 entry.

After installing a `--rust-engine` Waf build, run a bounded live map smoke
without copying the large asset corpus:

```sh
scripts/run_rust_hl2_smoke.sh /path/to/installed-runtime \
  "/path/to/Half-Life 2" 30
```

Use the `save-demo` scenario for the frame-synchronized acceptance path that
records and plays a demo and creates and reloads a save:

```sh
scripts/run_rust_hl2_smoke.sh /path/to/installed-runtime \
  "/path/to/Half-Life 2" 120 save-demo
```

The separate scripted endurance gate requires at least a 620-second timeout:

```sh
scripts/run_rust_hl2_smoke.sh /path/to/installed-runtime \
  "/path/to/Half-Life 2" 660 ten-minute
```

Use the sound-enabled intro gate to verify that the shipped G-Man sentence
phonemes reach live viseme and final model-flex output. Unlike the unattended
smoke/endurance paths, this scenario intentionally does not pass `-nosound`:

```sh
scripts/run_rust_hl2_smoke.sh /path/to/installed-runtime \
  "/path/to/Half-Life 2" 300 intro-lipsync
```

The gate samples both sides of that conversion across the sentence rather than
checking that each one happened once, because a face frozen on its first frame
emits the same one-shot markers as a face that animates; it requires the
viseme magnitudes and the rendered flex totals to each take a spread of
values. Its window covers the whole intro, since the scene does not begin at a
fixed offset from map load and a shorter wait passed the scene-start check
only when the timing happened to land.

The `physics` gate proves a player action changes the simulation: it spawns a
named crate under the crosshair, requires the solver to report it asleep, then
punts it with the gravity gun and requires the same object to be awake with a
nonzero velocity. It runs on `d1_trainstation_02` because the opening of
`d1_trainstation_01` holds the player in a scripted train pod:

```sh
scripts/run_rust_hl2_smoke.sh /path/to/installed-runtime \
  "/path/to/Half-Life 2" 240 physics
```

The `hud-audio` gate covers the presentation surface a player sees and hears.
It enables the `cl_hud_validate` and `snd_validate` cheat diagnostics, then
requires the rendered SMG clip to decrease as it fires, rendered suit power to
drain while sprinting, rendered health to drop from damage, a live audio device
rate, and the SMG shots themselves to reach the mixer. Like `intro-lipsync` it
runs with sound enabled:

```sh
scripts/run_rust_hl2_smoke.sh /path/to/installed-runtime \
  "/path/to/Half-Life 2" 180 hud-audio
```

The script clears the `gordon_invulnerable` global before asking for damage,
because HL2 legitimately holds it on through the opening chapter and the
player cannot be hurt until it is off. The mixer diagnostic names refused
starts as well as successful ones, since a run that plays no sound and a run
whose every sound is refused look the same from a count alone.

The 60-minute soak keeps live physics props present for the whole hour and
requires at least a 3660-second timeout:

```sh
scripts/run_rust_hl2_smoke.sh /path/to/installed-runtime \
  "/path/to/Half-Life 2" 3900 soak
```

The crates cover checked content paths, ordered loose/VPK search mounts,
stateful SDL input normalization, a bounded command queue and cvar registry, a
validated host lifecycle, deterministic bounded fixed-step scheduling,
byte/bit primitives, KeyValues/VMT, VPK, the ZIP archive a map embeds in
itself, BSP, VTF, Studio MDL/VVD/VTX/PHY,
WAV, scene images, demos, bounded server/client network messages, binary
DMX/PCF, and save/map-state containers, plus the beginnings of the permanent
presentation stack.

That renderer talks to Metal directly rather than through a portable graphics
layer, so it adds no dependencies to a workspace that has none: a small module
declares the four Objective-C runtime entry points it needs and wraps message
sending, and nothing outside it touches `objc_msgSend`. It opens the system
device and a command queue and runs a windowless pass that clears a
shared-storage BGRA target and reads the exact pixels back, which is the
capture primitive the visual comparison gates will be built on. On top of that
it compiles Metal Shading Language at runtime, builds pipeline state, and
draws, reading vertices either inline or from a buffer and sampling uploaded
textures; buffers and textures are owned by handles that release the Metal
object when dropped, so a resource's lifetime is the Rust value's lifetime.
Textures take BGRA8 or the BC1/BC2/BC3 blocks HL2's VTFs are stored in, and the
compressed payloads upload exactly as they are found on disk because Apple
silicon samples them natively. Frames are presented through a `CAMetalLayer`
that resizes and rescales for Retina, and a presented frame can be captured
out of the drawable it went to rather than from a second render that might
diverge from it. The layer is not yet handed to the running game's view: SDL's
GL context owns that view for ToGL, so the hand-off waits until the Metal path
can draw the game.

World geometry reaches the screen through a camera transform stated in
Source's own conventions rather than a graphics library's: `+x` is forward,
`+y` is left, `+z` is up, and positive pitch looks down, which is what the
engine already tracks and hands over. The projection maps that onto Metal's
`0..w` depth range instead of the `-w..w` one ToGL's shaders assume, and a
depth buffer decides which surface is in front, because a BSP stores its
faces in whatever order the compiler emitted them rather than back to front.
A pass takes as many draws as it has surfaces, since a world is drawn a batch
per material, and each draw reads its own run out of one shared index buffer
rather than owning a buffer of its own.

The textures those surfaces are drawn with come from the content as shipped.
A surface names a material, which is read through the same ordered search
path the engine builds, with the map's own embedded archive ahead of the
game's: that ordering is not a detail, because the cubemap materials the
compiler writes for each reflective surface's position exist nowhere else,
and they are 256 of `d1_trainstation_01`'s 397 materials. A material that is
a `patch` is followed to the one it derives from and then overlaid, so the
outermost patch wins. Its `$basetexture` is completed to the single lowercase
forward-slashed path the archives store, because shipped materials name
textures in whatever case the author typed and with either separator. The
`.vtf` it names uploads its whole mip chain, reversed on the way in: the file
stores the smallest level first, which is how the original loader could read
a reduced texture by stopping early, while a GPU texture numbers the largest
level zero. Nearly every texture is block-compressed and uploads exactly as
stored; the rest are widened to eight bits per channel, blue first, which is
the only uncompressed layout the renderer takes.

The world is lit by the map's own baked lighting rather than drawn flat. The
lighting lump holds a colour per luxel as three bytes and a shared exponent,
which is how it carries light brighter than white in four bytes, and the
high-range lump is preferred where a map has one: a map compiled for high
range stores both, and the standard-range copy is the dimmer fallback the
original hardware fell back to. Each lit surface's samples are packed into a
single image, because a map has thousands of surfaces carrying a handful of
samples each and binding a texture per surface would cost more than drawing
the world; blocks are packed tallest-first with a sample of padding between
them, so that filtering across a block's edge cannot reach into an unrelated
surface's lighting. A sample is written the way the engine writes it: divided
by the overbright, so a surface twice as bright as white still fits in a
byte, and in the screen's gamma rather than linearly, with the shader
multiplying the overbright back. Where a vertex falls in that image comes
from the surface's own lightmap projection, which is a separate pair of
planes from the texture's and at a far coarser scale. Lighting costs
vertices: two surfaces sharing a corner never share a lightmap block, so the
corner has to be stored once per surface rather than once per texture
projection, which takes `d1_trainstation_01` from 34,985 vertices to 41,440.
Only the static light style is read, because the later layers belong to
lights that switch or flicker and need the runtime to animate them; summing
them would light a lamp with its on and off states at once.

Its tests are the evidence, and they are written not to pass by skipping: on
Apple silicon a missing Metal device fails rather than excusing the run, a
draw has to change pixels away from the clear color, a texture is checked by
sampling four distinct texels into four quadrants so a flipped or rotated
upload cannot pass, and the compressed path decodes a hand-built block whose
endpoints expand to exact byte values rather than an approximation. A wall is
required to cover exactly the middle half of the view at the distance and
field of view that predicts, not merely to appear, and a nearer wall has to
win over a farther one in either draw order so that depth rather than
ordering is what is being tested. The BSP reader and the renderer are checked
against each other on a shipped map instead of synthetic geometry, because
only real content exercises whether the two agree about vertex layout, index
width, winding and scale: `d1_trainstation_01` triangulates into 24,854
triangles over 13,329 shared positions and fills the whole view from a point
the BSP itself reports as open space. The material chain is held to the same
standard twice over. Every one of the 5,193 materials Half-Life 2 ships is
parsed, and the texture path each produces has to be found in the archives,
which is the check that catches a path that is nearly right and would
silently texture the world with a fallback; all of them parse and 4,860 of
the 4,884 that name a base texture resolve, the rest being developer test
materials, HDR skybox variants and legacy HUD sprites. Then the whole chain
is drawn: `d1_trainstation_01`'s 397 materials bind 394 textures totalling
57.8 MiB, and the frame is required to come out in thousands of distinct
shades rather than merely to be covered, because a sampler given the wrong
coordinates or the wrong mip order leaves large flat areas that a coverage
count alone would accept. The three materials that bind nothing are the
camera feeds on that map's monitors, which the engine renders into rather
than loads. The lighting is held to a stricter standard than looking right,
because a lightmap is easy to sample wrongly in ways that still produce a
picture: coordinates that all land on one sample, or an image that decoded
uniform, would draw a textured world scaled evenly. So the same geometry is
drawn a second time through a pipeline that ignores the lighting and the two
frames are compared, from six places in the map and four headings at each.
Across those views the lighting has to darken most of what it touches and to
scale different parts of the world by widely different amounts, which it
does: between 0.00 and 0.94. The texture assertions are made on the unlit
frame rather than the lit one, so that a genuinely dark corner of a map
cannot be mistaken for a broken sampler. Those tests report that they were
skipped where no Half-Life 2 installation is present, since shipped content
cannot be redistributed; every other renderer test runs everywhere.

A frame draws what can be seen from where it stands rather than the whole
map. The map's visibility set says what a wall hides and the view's own six
bounding planes say what the screen does not hold; the planes are taken from
the transform being drawn with, so they cannot disagree with what is drawn.
Culling by the leaf a surface is listed in was tried first and is wrong: a
surface is listed in the leaves it starts in and reaches past them, on
`d1_trainstation_01` by as much as 1,440 units. Each surface carries its own
bounds instead. Over twenty-four views taken from the map's own player
starts and entity positions, a frame draws 4.5 percent of its triangles on
average and 27.9 percent at worst, in 1,863 draws rather than 8,184.

Culling is held to the standard that it changes nothing. Every view is
rendered twice, culled and whole, and no pixel may move by more than one
step. That is a bound rather than an exact match because the bound was
measured: drawing the same triangles in the same order as more draw calls
than before already shifts the odd pixel by one, and culling splits a
material's run wherever the surfaces between it were dropped. A hole, by
contrast, shows whatever was behind the surface, which never lands within a
step of it.

A map's other brush models are drawn too. These are its doors, lifts,
breakables and trigger volumes, each cut out of the world by the compiler
and stored about an origin of its own, so drawing them where they are
stored heaps them on the map's origin. The entity that owns a model carries
the origin and angles that put it back, and a model whose entity gives no
origin is already in world coordinates, which the same transform covers by
reading as the identity. Only the position is transformed: the texture and
lightmap projections are defined against the surface as compiled, so a
placed door keeps the texture and baked light it was built with. The
visibility set does not describe these surfaces, because the leaves list
them where they are stored, so a placed model is culled by the leaf it
stands in rather than by the leaves that list it. Triggers drop out on
their own, since a surface whose texinfo is not drawn never reaches the
geometry.

All of that now draws in the running game rather than only in a test, with
no OpenGL device anywhere in the process. The engine's need for a graphics
device and its need for one that actually draws turn out to be separate,
and only the first has to be met for the game to run: under
`-metal -noshaderapi` the launcher loads `shaderapiempty` instead of
`shaderapidx9`, so ToGL is never loaded and no GL context is created, and
the window carries a `CAMetalLayer` rather than a GL surface. The engine's
frame loop then draws the map through this renderer, where the old renderer
drew, once per frame with the view it has just set up.

The seam between the two is two calls wide. The engine hands in a position
and a set of angles and gets back a presented frame; it says nothing about
batches, lightmaps or culling, and it is asked nothing about entities or
game state. The map, its materials and its packed lighting are loaded onto
the Metal device once, and each frame selects from them by the map's own
visibility set and the view's bounding planes. On `d1_trainstation_01` that
is 23,999 triangles across 397 bound materials, of which a frame draws
between 445 and 7,686 depending on where the player is standing.

What a run drew can be kept by setting `SOURCE_METAL_SHOT_DIR`, which
writes the presented drawable's own pixels rather than a second render of
the same view, so what is looked at afterwards cannot differ from what was
shown. Models, particles and UI are still not drawn, and the engine's own
level-load screen is still the old renderer's; neither reaches the display,
because the display is the Metal layer.

The pieces a model needs are read, though not yet drawn. `source-studio`
decodes a model's vertices through the level-of-detail fixup table, which is
what a file stores instead of one run of vertices per level: the levels share
most of their vertices, so the file keeps one run and a table of which
stretches each level takes. Reading the run directly gives the right count and
the wrong vertices. Triangles come from joining two files, because neither
holds the answer on its own: the triangle file numbers each mesh's vertices
from zero and says which of them each triangle uses, and the model file says
where that mesh sits in the single run the vertex file stores. Strips are
walked with their alternating winding and their joining degenerates dropped.
Material names and the directories to look for them under are read too, with
the separators normalised, since a model compiled on Windows names its
directories with backslashes.

A model's vertices are stored in model space, in the rest pose, whether it
is jointed or not. What varies is whether the box the compiler wrote into
the header is a bound on them: for a static prop it is, which is the oracle
the shipped-model test holds the decode against, and for a jointed model it
is the movement hull instead, so a person's arms reach past their 13-by-13
walking box and a door's hull is the volume it sweeps rather than the leaf
it is.

The skeleton is composed all the same, because an animated pose is stated
relative to the rest one, and because it is the only statement a model file
makes about itself that can be checked without a second source: the
compiler writes each bone's transform out of model space as the inverse of
where that bone sits at rest, so composing from the root down and
multiplying the two must leave every vertex where it was. Nothing else
catches a quaternion read in the wrong component order, a matrix composed
the wrong way round, or a bone hung off the wrong parent — each of those
still yields a skeleton, and each places a limb somewhere plausible. Every
model the base game ships is held to it rather than a handful, because the
skeletons that catch those mistakes are the deep ones with rotated joints
and the opening map's props are mostly single bones at the origin, which
cancel whatever order they are composed in. Of the 2,201 models the
archives list, 2,195 compose, 13,197 bones and thirteen joints deep at the
most, all cancelling to within 7.3e-4; the other six are names the archives
hold nothing under, which the test asserts are empty rather than skipping
whatever fails to parse.

`source-bsp` reads the static prop lump the compiler writes inside the game
lump, which is where a map keeps the crates, lamps and railings it is dressed
with rather than in the entity lump. Its offsets are into the whole file
rather than into the lump holding it, and its record stride depends on the
lump's version, which is the part that goes wrong quietly: a wrong stride
still yields props that look plausible until they are drawn. On
`d1_trainstation_01` that is 298 props over 86 models, every one standing in
a leaf a view can reach.

Those props are drawn. Each dictionary entry is loaded once and placed as
many times as the map places it, which is the reason the dictionary exists,
and the placements are checked against each model's own reach rather than a
fixed distance, since a lamp post's geometry stands a long way above the
point it is placed at. Of `d1_trainstation_01`'s 298 placements, 284 over 82
models draw, the rest naming models the base game does not ship; that comes
to 164,447 triangles in 50 material runs, every one binding a shipped
texture. The frames are held to the shape a right answer has rather than to
coverage: props stand about a map rather than covering it, so all four views
from the player's start hold some without any being filled, which is what
geometry heaped around the player's feet would draw, and the best draws
8,852 distinct shades, where a handful would be the flat fill a lost
coordinate buffer leaves.

Those props are lit by the map. A world surface has a lightmap because the
compiler knew where it was and which way it faced; a prop does not, so the
compiler records the light arriving from each of six axial directions at
points inside every open leaf instead, and a surface takes whichever of
those it faces, weighted by the square of each component of its normal so
that turning a surface neither brightens nor darkens it. Half-Life 2's own
maps keep that cube inside each leaf record rather than in the lumps a
later compiler writes it to, so reading only those lumps finds them empty
and lights every prop in the shipped campaign black; both are read, the
later lumps preferred where a map has them.

There is no picture to look at that would show this being read wrong, since
a cube whose faces are in the wrong order or whose shared exponent is
applied wrongly still shades a prop to some plausible grey. It is held to
what must be true of light in a room whatever the room is: of
`d1_trainstation_01`'s 5,353 leaves, 5,283 carry a measurement, 3,989 of
the 5,242 that differ top to bottom are brighter above than below, and the
peaks run from 8.4e-9 through a median of 6.8e-3 to 1.23, a spread of eight
orders of magnitude that a constant does not produce. This is the ambient
term only: Half-Life 2 shipped before the compiler baked per-prop vertex
lighting, so the engine adds the map's direct lights to this cube at
runtime, which is not done here and leaves props dimmer than the world.

Run the installed-content gate without copying Steam assets into the
repository:

```sh
cargo run --release -p source-content-audit -- \
  --known-defects docs/rust-port/hl2-known-content-defects.txt \
  "/path/to/Half-Life 2"
```

Use `--loose-only` with a runtime staging tree to validate generated saves and
other loose artifacts without rescanning its copied VPKs.

Parser entry points also have `cargo-fuzz` targets under `rust/fuzz`. CI now
builds every target and gives each a short run, because a manifest that parses
says nothing about whether the target still compiles, and a target that no
longer compiles is a format that is no longer being fuzzed. Building them
locally needs nightly:

```sh
cargo +nightly fuzz build --fuzz-dir rust/fuzz
cargo +nightly fuzz run --fuzz-dir rust/fuzz network -- -max_total_time=120
```

The short CI runs are a regression check, not a campaign; sustained fuzzing
remains a separate activity.

On macOS, the `--rust-engine` Waf install now uses the Cargo-built
`source-launcher` as `hl2_launcher`. Rust therefore owns the OS process entry,
preserves the exact Unix argument bytes, resolves `bin/liblauncher.dylib`
relative to its installed executable, and invokes `LauncherMain` as one
transitional callback. The old C++ launcher executable is not built in this
configuration.

`CAppSystemGroup::Run` and the separate `Startup`/`Shutdown` API now delegate
their startup/rollback sequence to `source-host::app_system`. Rust tracks completed
connections and initializations, shuts those down in reverse order, runs the completed PreInit
phase's cleanup, disconnects before unloading, and preserves Main's exit result.
Native callbacks still execute each subsystem operation and module IO; failed
callbacks must unwind their own partial work. Split startup returns a Rust-owned
handle; shutdown requires the original thread and callback/data identity. Stale,
duplicate and reentrant operations are rejected before callback dispatch, and no
registry lock spans native callbacks. The ABI gate checks 1,088 startup/failure
cases for each API, ownership guards and 1,000 reentrant split lifecycles.
An installed-runtime failure-path gate is available:

```sh
RUST_GAME_SCENARIO=init-failure sh scripts/run_rust_game_smoke.sh \
  out-rust-allgames hl2 d1_trainstation_01 90
```

It injects shader-device initialization failure after connection and requires
both groups to clean up, with error exit 255 rather than a crash or a successful
startup claim. Use `RUST_GAME_RENDERER=togl` for the retained native renderer.
Add `RUST_GAME_SPLIT_STARTUP=1` to exercise separate startup/shutdown, including
a second shutdown call that must not dispatch cleanup again. Without that flag,
the launcher uses synchronous `Run` as before. The flag also works for the normal
fixed-camera map-load/exit gate; neither gate establishes NPC speech correctness.

The Cargo build is exposed to Waf with `--rust-engine`; its host context first
mounts the selected game's `gameinfo.txt` GAME paths, then rebuilds that
registry whenever the native filesystem changes its ordered loose/VPK/ZIP/BSP
search paths. Rebuilds retain cached validated archive indexes,
preserve by-request-only visibility, and follow the trusted runtime content
symlinks with native semantics. ZIP/BSP ranges are discovered, opened and
validated by Rust, including stored and LZMA entries. Native code still owns
mount registration and head/tail placement, consuming Rust-discovered numbered
candidates and archive metadata in its search-path registry. The old
legacy-pack precedence bypass is removed.
The native `CZipPackFile` adapter now also delegates directory/local-header
validation, canonical filename lookup, entry offsets/sizes and index lifetime
to a metadata-only Rust `source-pak::Index`. The native hash table and central
directory parser are excluded from Rust builds. An opaque handle owns names and
metadata after the borrowed input buffer is released. The optional native
preload cache is bypassed in favor of archive payload bytes, and its reserved
`__preload_section.pre` entry is hidden from this index. Absolute/filtered pack
opens now use Rust payload reads too, and pack wildcard matching now runs in
Rust. This does not retire the entire pack adapter.
`source-filesystem::pack_archive::Archive` owns the validated metadata and
payload source. File-backed archives retain the descriptor but release temporary
ZIP bytes. `source_context_read_path_add_pak_index` shares that owner with
ordinary search mounts, without path lookup, reopening, reparsing, or retaining
a second compressed-byte copy. Both ordinary and explicit pack reads therefore
use the same original descriptor after pathname replacement. New remounts see
the replacement; in-place corruption/truncation fails CRC/IO checks rather than
falling through to a lower-priority file. This is not an atomic file snapshot.
Index handles, mounts in multiple contexts, and decoded file cursors have
independent lifetimes. The last archive reference closes the descriptor;
decoded files remain usable without retaining it.

Read-path selection now shares `source-filesystem::selection::path_id_matches`
between all Rust mount queries and the remaining native search iterator. No
requested ID excludes by-request-only mounts; an explicit ID bypasses that
flag and matches ASCII-case-insensitively. `BSP` is reserved for GAME map packs,
not loose directories, VPKs, standalone ZIPs or a literal BSP-ID mount. Typed
archive provenance is retained instead of inferred from a filename or offset.
Legacy untyped byte-range mounts do not qualify; the in-memory map-pak API does.
The context-free `source_read_path_matches` ABI distinguishes an absent request
from an explicit empty one and fails closed on invalid flags/slices. Existing
context read/find APIs retain their empty-slice-means-unspecified contract.
Native registration/reuse decisions and trust checks remain; table mutation
and resource lifetime ownership are described below.

`source-filesystem::search_plan` owns the remaining iterator's selected order,
position and store-ID deduplication. The context-independent plan ABI consumes
borrowed mount descriptors and retains only source indices. Filtering by
path/type/platform exclusion happens before visiting an opaque signed store
ID, allowing an eligible alias to win after an earlier alias was rejected.
VPK remains non-pack for the native `FILTER_CULLPACK` contract; only ZIP/BSP
count as pack paths there. Plans freeze selection metadata for one traversal;
the C++ GetFirst adapter builds a fresh plan, while the ABI reset replays the
same immutable plan. Native resource references remain in the iterator's
existing snapshot; returned Rust indices select those references. Empty and
absolute pseudo-path handling stays native.

The fallback native find adapter also uses Rust-owned store visits, replacing
its C++ visited-ID vector. Opaque handles own either a plan or a visit set;
wrong-kind/stale handles fail, concurrent calls serialize, and C++ destructors
release the handles. No active engine context is needed during startup/unload.
The plan ABI limits snapshots to 65,536 entries and UTF-8 IDs to 4,096 bytes.
Xbox exclusion-name lookup supplies an input flag from native code; its ordering
is unit/ABI/fuzz tested, not Xbox-runtime validated. Native registration policy
and public resource/file/find adapters still remain.

The ordered mount table and resource-lease lifetime now belong to Rust too.
Both the live native filesystem table and its iterator snapshots use
`source-filesystem::mount_table`, not a C++ ordered vector. A thin facade maps
legacy index access to opaque table handles. Temporary CSearchPath objects are
opaque host contexts, with explicit clone/drop callbacks: insertion transfers
ownership only on success; ordered removal, swap removal, clear and destruction
release resources in Rust-controlled lifetimes. Snapshot callbacks copy headers
and AddRef existing pack/VPK resources, preserving the previous copy semantics.
Source leases remain pinned during callbacks, including reentrant source-table
destruction. A failed clone drops earlier copies without publishing a partial
snapshot. Callbacks run outside both handle-table and per-table locks.

Callback code/data must outlive all owning tables and must support the calling
thread without unwinding. Borrowed entry pointers require caller serialization
against removal; this is not an unrestricted concurrent native-pointer API.
Registry snapshots are bounded to 65,536 mounts. The monotonic positive store-ID
allocator is now Rust-owned and fails at exhaustion rather than wrapping.
Native physical-path alias matching, duplicate registration/repositioning rules,
map CRC identities, path-ID metadata, archive reuse and trust still remain.
Those metadata/policy decisions are the next boundary; the table's opaque
native-resource callbacks are transitional, not full adapter retirement.

The explicit-range `source_pak_index_open_file` and ordinary range-mount Rust
ABI remain available for compatibility, using the same archive implementation.
The native adapter no longer calls a range-mount bridge or stores the archive
length/base offset in Rust builds. Three unused private C++ bridges for raw
index creation, range opening and range mounting were removed. The public Rust
ABI constructors remain; slice-created indexes are metadata-only and cannot be
mounted or open payloads. The Rust memory-pack API still owns its supplied ZIP
bytes. All pack modes hide the reserved preload-cache entry from content reads,
metadata queries and enumeration. Empty in-bounds ranges represent empty packs.

The native mount adapter now uses `source_pak_index_open_archive`, specifying
standalone ZIP or embedded BSP ZIP rather than a byte range. Rust reads the
fixed BSP header (versions 19..21), rejects negative/out-of-file/header-overlap
ranges and outer-compressed pack lumps, and treats an empty BSP lump as absent.
The same descriptor supplies the range, timestamp, index and payload reads.
Other BSP lumps are not validated by this mount operation; full `Bsp::parse`
still validates all ranges through the shared header parser. Native header
reads, ZIP length seeks and mount-open handles are excluded from Rust builds.
Numbered ZIP discovery now uses `source_context_find_pack_candidates`: Rust
snapshots full candidate paths, higher numbers first, with optional Xbox360
localized series ahead of the base series. A failed metadata lookup terminates
that series, matching the old stat policy; a present malformed archive does not
hide later candidates. Symlinks are followed, directories remain candidates
until archive validation, and paths are not truncated. Discovery retains names,
not file descriptors, so the snapshot does not promise atomic file contents.
The context find-next/close APIs own its lifetime; short output does not publish
or advance a cursor. Invalid paths/languages and more than 65,536 archives per
series fail without a partial mount. Xbox naming is covered by pure/ABI tests,
not Xbox runtime validation. Mount registration/reuse, path-ID trust, and the
head/tail placement of loose directories remain native.
The loaded native filesystem gate also exercises 200 BSP replacement/remount
cycles, nested BeginMapAccess/EndMapAccess, refreshed GAME/BSP lookup and glob
results, and old-map decoded file handles surviving replacement and unmount.
These are filesystem lifetime checks, not live campaign-transition coverage.

`source_context_find_first_pak` snapshots index matches into a context-owned
cursor, using the ordinary Rust filesystem's ASCII-case-insensitive bytewise
glob matcher (`*`, `?`, Win32-style `*.*`). It returns canonical relative names,
sorted files before sorted implied directories, without duplicates; files win
same-name directory collisions. Archive-local dot/slash normalization is allowed
but root escape, absolute paths, wildcard directories and empty basenames fail.
Short buffers do not consume results, and a cursor survives index destruction.
The fallback pack adapter drains these results into its existing FindData
lists; public native list iteration remains, not a second matcher.
Unlike the old pack-only matcher, root files, partial wildcards and multi-dot
names work consistently with ordinary Rust finds. Relative ZIP finds now also
match directory prefixes case-insensitively and hide the preload-cache entry.
The Rust ABI exposes full validated names up to 1024 bytes; the native adapter
omits basenames that cannot fit its legacy MAX_PATH field instead of truncating.
Ordinary BSP-only queries now use selected Rust map mounts and the same
archive-local normalization. `source_context_find_first_bounded` filters long
names before publishing a cursor; its query limit is independent of output
capacity, so short buffers still do not truncate or consume results. Native
BSP queries keep the legacy MAX_PATH limit even where POSIX FindData is wider.
The original unbounded find ABI is unchanged. BSP text opens retain the native
CRLF-aware pack adapter; their payload and cursor still belong to Rust.

`source_context_file_open_pak` shares the bounded stored/LZMA decoder with
ordinary relative reads, with a 64 MiB dictionary and 256 MiB decoded-entry
limit. Seeks clamp to [0,size], including negative seeks (fixing the native
stored reader's unsigned wrap to EOF); invalid origins leave the cursor alone.
Opened payloads survive index destruction/unmount. A thin `CPackFileHandle`
adapter preserves native text-mode CRLF handling without owning a cursor or
decoding bytes. `CZipPackFileHandle`, `CLZMAZipPackFileHandle` and `ReadFromPack`
are excluded from Rust builds. Full payload materialization on open can use
more memory than the former native streaming decoder; campaign-scale memory
and latency remain unmeasured.

The public ABI metadata differential also checks payload offsets against Python
(including distinct local/central extra fields), 10,000 index lifecycles, stale
handles, short buffers and concurrent queries. To exercise the actual installed
filesystem module without game content, including stored/LZMA ZIP and BSP
absolute reads, seeks, BSP enumeration and module cleanup:

```sh
sh rust/verify_native_pak.sh out-rust-allgames
```

This macOS gate builds the native LZMA SDK only as a fixture oracle and is wired
into the HL2 CI matrix entry. It links the bridge's ABI instance so its context
and the filesystem share one handle registry.
Rust performs bounded CRC-validating search-path reads and
validates its scene-image cache during startup. Ordinary relative opens across
the synchronized path IDs now return context-owned opaque Rust handles. Loose
files remain streaming disk handles; VPK/ZIP/BSP entries become CRC-validated memory
cursors. Read, seek, tell, size, open-state, and close remain behind the existing
pointer-shaped `IFileSystem` adapter, with legacy fallback for unmounted content
and absolute/pure-server/pack-filtered paths. Corrupt recognized relative entries
do not fall back to native reads. Positive relative `FileExists` and
filename-size queries use the same Rust registry without opening or copying the
asset. Relative `IsDirectory` queries cover both canonical loose directories
and virtual VPK/ZIP/BSP directory prefixes. Relative
`FindFirst`/`FindNext`/`FindClose` calls use context-owned Rust wildcard cursors
that preserve mount precedence, filter by path ID, suppress duplicate names,
and synthesize immediate archive directories. Absolute enumeration retains its
native fallback; BSP selection is shared with Rust. For a relative write, Rust
maintains a separate ordered registry synchronized from
the native loose-directory search paths and selects `GAME_WRITE`, `MOD_WRITE`,
the requested path ID, `DEFAULT_WRITE_PATH`, or the first directory using the
legacy precedence rules. Rust returns the canonical target used by write-open,
directory creation, and removal. Relative write-open then returns an
opaque context-owned Rust file ID; read, write, flush, seek, tell, live size,
open-state, and close operations remain in Rust behind the existing
`IFileSystem` adapter. Rust also performs relative recursive directory creation
and file removal against that registry, searches synchronized loose paths for
relative rename sources and writability queries, and owns relative permission
changes and renames. Absolute/platform mutations remain native. For a
`+map` launch it also owns and validates the BSP's
plane/node/leaf tree and PVS/PAS rows. `SOURCE_HL2_CONTENT_ROOT` can identify a
canonical external asset tree when the installed runtime keeps content on
symlinks. Resolved paths keep the spelling of the search path they were found
under rather than the symlink-resolved location, because the engine attributes
an absolute path back to a search path by prefix; symlinks are still resolved
internally to detect a traversal escaping a mount. The launcher advances the Rust host lifecycle around the still-legacy
engine and delegates bounded outer-session restart control to Rust. Individual
frame/message iterations and their stop/restart decision are also controlled by
a Rust loop. A context-owned Rust frame pacer accounts elapsed time and decides
whether each render frame is ready or must wait; the transitional callback
still pumps platform messages and performs the requested platform sleep. Rust
also owns the live simulation-tick accumulator, including fractional remainder,
paused-demo accumulation, and single-player alternate-tick alignment; C++ still
executes the selected legacy tick bodies. Every server map spawn synchronizes the validated
Rust-owned BSP world and each frame drains a bounded Rust-owned command queue
into the transitional executor. New-game, save-load, change-level, shutdown,
and restart requests occupy a validated Rust-owned next-operation slot before a
C++ adapter executes the remaining legacy map/session body. Live network
channels also register Rust-owned outgoing sequence and incoming
duplicate/order/drop state. Rust emits and finalizes the outgoing
sequence/ack/flags/reliable/choke/challenge header, supplies the folded packet
CRC when checksums are enabled, and parses the same incoming fields through a
bounded validator before the retained native fragment logic runs. The native
adapter still owns payload assembly, split-datagram transport, reliable fragments,
and message dispatch. Generic engine-buffer compression/decompression now uses
Rust's LZSS and Source-tagged Snappy codecs, with no native error fallback.
Snappy compatibility is cross-decoding, not byte-identical compressed output;
the ABI gate tests both directions against the retained native oracle. Tagged
malformed streams fail instead of being copied as raw data. Rust builds omit
the native Snappy translation units from tier1. The UDP and reliable-fragment
receive paths now honor decompression failure before parsing output.
Networked string tables
use Rust-owned canonical strings, case-insensitive indices, bounded user data,
change ticks, and rollback history, while native dictionaries remain
pointer-stable mirrors and retain callbacks. Rust also encodes those entries
into the bit layout demo and save containers carry, leaving the native writer
only the client-side section that follows, which is not table state. Rust also owns
validated server datatable/property metadata, native-compatible CRC calculation,
and canonical server-class IDs. Immutable server-frame entity/serial/class
ordering and explicit-delete state live in Rust-owned snapshot records; the
native side retains pointer-bearing mirrors, proxy callbacks, entity packing,
and change detection. Rust decides what each receiver's packet-entity delta
contains: given the two snapshots narrowed by the receiver's visibility sets, it
classifies every differing entity index as a creation, a removal, a recreate, or
a delta candidate, and it rejects an inconsistent visibility set outright. Rust
also encodes the per-entity delta header bits and the 1072-byte demo file
header, so the bytes a recording emits come from the same implementation that
parses them back. Save containers are composed the same way at both levels:
Rust frames the `VALV` map state from its tag, four section sizes, token table
and two data sections, and frames the leading `JSAV` save from its tag,
version, sizes, token table and game data, leaving only the embedded map-state
copy to the caller. Both refuse a container the Rust reader would reject, so an
inconsistent set of sections is caught where it is produced rather than at load
time. The native writer still compares packed payloads to choose
between resending and preserving a delta candidate, and still emits the payload
bits themselves. Waf remains the top-level transitional orchestrator while
legacy modules are still present.

Every one of these encoders keeps a native fallback that engages only if Rust
refuses or disagrees with the native writer, and each reports that on the
console; the string-table encoder additionally refuses when its entry count
disagrees with the native dictionary, since the two are meant to mirror each
other. Falling back rather than failing matters most for the save container,
because a refused transition save would lose level state outright. The
`save-demo` gate requires all of them to report themselves active and fails if
any fell back, so a silent regression to the legacy path cannot pass. Because single-player
normally hands entity state to the client in process, the acceptance script
turns `cl_localnetworkbackdoor` off to make the listen server encode real
packets.
