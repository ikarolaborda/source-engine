# Direct3D 9 on Metal

The route to a playable game through Metal. It replaces ToGL, not the
material system: `shaderapidx9` keeps issuing the Direct3D 9 calls it always
has, and a Rust device turns them into Metal instead of ToGL turning them
into OpenGL. Everything the material system can draw, which is the whole
game, reaches the screen through it at once, rather than one renderer
feature at a time.

The from-scratch world renderer under `-metal -noshaderapi` stays in the
tree. It is no longer the path to playable: it draws the map and its static
props and nothing else, and every remaining feature of the renderer would
have had to be rebuilt behind it.

## Shape

```
shaderapidx9 sources, compiled as shaderapimetal with -DTOMETAL
        |  IDirect3DDevice9 and friends (public/tometal/dxabstract.h)
        v
tometal/           thin C++ classes, no state worth the name, no ObjC
        |  C ABI (source_d3d9_*), public/rust/source_abi.h
        v
rust/crates/source-d3d9
        translate.rs   D3D9 shader bytecode -> Metal Shading Language
        device.rs      state tracking, pipelines, encoders, presentation
        mtl.rs/objc.rs the Metal binding, no external crates
```

`-metal` selects `shaderapimetal`; the window is created without
`SDL_WINDOW_OPENGL` and carries a `CAMetalLayer`. Without `-metal` nothing
changes and ToGL remains the oracle.

## Conventions the translator and the device share

Entry points are `vs_main` and `ps_main`. Every translated library starts
with the same prelude, which declares:

```metal
struct D3DExtra {
    int4   ic[16];        // integer constants i0..i15
    float4 clip_plane0;   // clip-space plane; all zero when disabled
    float4 pos_fixup;     // x = 1/viewport_w, y = -1/viewport_h
    uint   bools;         // bit n is bool constant bN
    uint   alpha_func;    // D3DCMP_* 1..8
    float  alpha_ref;     // 0..1
    uint   flags;
};
constant bool fc_alpha_test [[function_constant(0)]];
```

Vertex stage: streams are `buffer(0..7)` through the vertex descriptor, and
attribute `n` is D3D input register `vn`. Float constants are
`constant float4 *vc [[buffer(16)]]`, extras `constant D3DExtra &ex
[[buffer(17)]]`. Vertex textures and samplers take their register number.

Pixel stage: `constant float4 *pc [[buffer(0)]]`, `constant D3DExtra &ex
[[buffer(1)]]`, `texture(n)`/`sampler(n)` for `sn`.

Interpolators are matched by semantic, never by register, so that SM2 and
SM3 shaders link with each other and with themselves: `[[user(texcoordN)]]`,
`[[user(colorN)]]`, and `[[user(uU_I)]]` for any other usage `U` index `I`.
SM2 `oTn`/`tn` are texcoord n, `oDn`/`vn` are color n. A vertex function
always declares and zero-fills the whole fixed set (color 0-1, texcoord 0-9,
fog) plus anything else it writes, because Metal refuses a pipeline whose
fragment function reads an interpolator the vertex function lacks. A pixel
function declares only what it reads.

Vertex epilogue, in this order: `clip0 = dot(pos, ex.clip_plane0)` into
`[[clip_distance]]`; then `pos.xy += ex.pos_fixup.xy * pos.w`, which is
Direct3D 9's half-pixel centre convention. Depth is already `0..w` and y is
already up, so neither is touched.

Pixel epilogue: when `fc_alpha_test`, compare `oC0.a` against
`ex.alpha_ref` by `ex.alpha_func` and `discard_fragment()` on failure. It is
a function constant rather than a uniform because a fragment function that
can discard loses early depth on a tile-based GPU whether or not it does.

sRGB is not the translator's business: reads bind the texture's sRGB view,
writes attach the render target's.

## What draws, and how it was checked

Everything the material system draws, because nothing above the Direct3D
calls changed: the menus and console, VGUI and the HUD with legible text,
the world with lightmaps and bump mapping, displacements, static and
physics props, skinned and flexed characters, view models, vehicles, water,
fog, sky, captions and the intro's blended scenes. Frames taken at the same
place on `d1_canals_03` and `d2_coast_01` through ToGL and through this are
the same picture apart from ToGL's multisampling.

The shipped shaders are all `vs_2_0` and `ps_2_b`. The translator was run
over every shader five maps create, 1,325 of them, and every translation
compiles; the corpus is rebuilt by running the game with
`SOURCE_SHADER_DUMP_DIR` set, and checked with

```sh
SOURCE_SHADER_CORPUS=<dir> cargo test -p source-d3d9 --test translate_corpus -- --nocapture
```

`SOURCE_D3D9_SHOT_DIR=<dir>` has the device write its own back buffer there
every `SOURCE_D3D9_SHOT_EVERY` frames (600 by default) as BMP. That is how a
run is looked at: it is the game's pixels and nothing else that is on the
screen, and it does not depend on the window being in front.

## Running it

```sh
bash scripts/build-macos-arm64-rust.sh
./hl2_launcher -game hl2 -metal -fullscreen -nativeres
```

`-metal` selects `shaderapimetal`. `-nativeres` sets the back buffer to the
display's own pixel count, which on a display whose points are not pixels
is not the size the window manager reports for the desktop: a 14-inch
MacBook Pro reports 1512x982 and has 3024x1964 pixels, of which a
full-screen window covers the 3024x1898 below the camera housing. The mode
list offered to the video settings dialog is in pixels for the same reason,
and the mouse is scaled from the window's points to the back buffer's
pixels by whatever the window currently is.

Existing gates take the path with `EXTRA_LAUNCHER_ARGS="-metal"`.

`rust/tests/hl2_metal_tour.txt` is a test script for looking rather than
asserting: the flashlight in a dark tunnel, gunfire, a spawned character, then
a map with fire and one with water. Copy it into `hl2/testscripts/`, run with
`-testscript hl2_metal_tour.txt +map d1_canals_03` and a shot directory, and
read the frames. A window that is not in front runs at about twenty frames a
second, which is the engine sleeping, not the renderer.

## Details worth knowing before changing it

- **Enumerations are the engine's, not Direct3D's.** On POSIX the engine
  declares `D3DFORMAT` itself and numbers it from zero
  (`public/bitmap/imageformat.h`), and ToGL's headers number the texture
  address modes from zero with no mirror modes. Render states, compare
  functions, blend factors and declaration types are Direct3D's own.
- **Vertex colours are red-first.** `imesh.h` is compiled with
  `OPENGL_SWAP_COLORS` everywhere, so `D3DDECLTYPE_D3DCOLOR` is read as plain
  normalized bytes, not as BGRA.
- **A few formats are widened on upload.** `R8G8B8`, `L8` and `A8L8` have no
  Metal equivalent and the engine creates small textures in them without
  asking, so they are stored as BGRA. `X8R8G8B8` has its alpha set on upload
  and cleared to one, since Metal has no format with an ignored channel.
- **A whole-target clear becomes the next pass's load action** rather than a
  pass of its own, and its colour is decoded first when the pass writes
  through an sRGB view, because Direct3D's clear ignores sRGB write and
  Metal's does not.
- **Shadow-map samplers are found from state, not from shader names.** The
  engine sets `D3DSAMP_SHADOWFILTER` on them. A pixel shader is translated
  again with those samplers as `depth2d` and `sample_compare` the first time
  a draw binds a depth texture under one.
- **Occlusion queries** count into a visibility buffer owned by the command
  buffer they were drawn in and are answered once it completes. On OS X the
  client only uses them when `gl_can_query_fast` says so, which `tometal`
  declares as 1.
- **Shaders compile on first use**, and pipelines are built on first use of
  each combination of shaders, vertex layout, target formats and blend
  state, so new scenes hitch briefly the first time they are seen.

## Not done

- Multisampling. ToGL ran at 8x; this reports none. At the display's native
  pixel density the difference is small but visible on wires and fences.
- A cache of compiled shaders and pipelines across runs, which is what
  would remove the first-sight hitches.
- Vertex textures, triangle fans, `DrawIndexedPrimitiveUP`, scaled copies
  from a mip level, and copies between depth surfaces. None has been seen
  in play; each logs once if it is.
- Texture uploads and buffer renames assume no more than three frames in
  flight, which the layer's three drawables guarantee today.

