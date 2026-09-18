# Native macOS Metal renderer follow-on

This is the permanent presentation stack the port plan sequences immediately
after the Rust-owned engine host, and before simulation, the toolchain, and
bridge retirement. It is not part of the current stabilization soak, and the
legacy ToGL path remains the compatibility oracle until the replacement earns
parity.

## Direction

- Build a Rust-owned renderer with a native Apple-silicon Metal backend rather
  than extending the deprecated OpenGL/ToGL adapter.
- Reach Metal directly through a small Objective-C runtime binding kept inside
  the renderer, not through a portable graphics abstraction. This keeps the
  workspace free of external dependencies and puts nothing between the engine
  and the platform API; the cost is that Windows and Linux need their own
  backends written against the same interface during platform expansion.
- Preserve existing HL2 maps, models, materials, particles, UI composition,
  facial morphs, and scene timing at the content boundary.
- Improve output quality deliberately: correct linear/sRGB composition,
  higher-quality shadow filtering, modern anti-aliasing, anisotropic material
  sampling, stable water/reflection rendering, improved tone mapping, and an
  optional HDR/EDR presentation path where the display supports it.
- Keep enhancements separable from compatibility rendering so a content or
  shader regression can be isolated from an intentional quality change.

## Delivery slices

1. Establish a native Metal device/swapchain, resize/fullscreen/Retina handling,
   validation labels, frame capture, and deterministic offscreen readback.
   Device ownership, a command queue, deterministic offscreen readback, a
   `CAMetalLayer` swapchain with resize and Retina scale, presentation, and
   capture of the drawable that was actually presented all exist. Validation
   labels, fullscreen transitions, and display sleep/wake recovery do not.
   The swapchain is deliberately not attached to the running game's view yet:
   SDL's GL context owns that view's layer for ToGL, and replacing it would
   break the renderer that is still the oracle. Attachment therefore lands
   with the ToGL hand-off in slice 5, and until then the layer is exercised
   standing alone, which is also what makes capture possible without a window.
2. Port resource lifetime, texture formats, buffers, command submission, and
   offline Metal shader packaging behind Rust-owned handles.
   Runtime shader compilation, render pipeline state, vertex buffers,
   sampleable textures, and drawing through them exist, each owned by a handle
   that releases on drop.    Textures take BGRA8 and the BC1/BC2/BC3 blocks HL2's
   VTFs are stored in, which upload unchanged because Apple silicon samples
   them natively, and a whole mip chain uploads as the material author shipped
   it rather than being regenerated on the GPU. Index buffers take the 16-bit
   entries Source stores, with the indices checked against the vertices they
   select from before a draw is encoded, since an out-of-range index faults
   the GPU rather than drawing the wrong thing. A draw reads its own run out
   of an index buffer rather than the whole of one, which is what lets a
   map's world be one buffer grouped per material. Per-vertex texture
   coordinates bind as their own buffer, because that is how a map stores
   them. A pass takes as many draws as
   it needs rather than one, because a world is drawn as a batch per material.
   Offline `.metallib` packaging, the remaining VTF formats, and cube maps do
   not exist.
3. Render BSP visibility/world geometry and the legacy material baseline.
   World surfaces now parse out of the vertex, edge, surface-edge, face, and
   leaf-face lumps and triangulate into indexed geometry that shares the
   corners the format shares, with every index checked against the lump it
   points into at parse time rather than during traversal. Reversed surface
   edges are walked in the direction their sign gives, so a polygon comes out
   wound as the compiler wound it, and displacement faces are skipped because
   their geometry lives in a separate lump. That geometry now draws through
   the Metal device: world positions are placed on screen by a camera
   transform stated in Source's own conventions, where `+x` is forward, `+y`
   is left, `+z` is up and positive pitch looks down, mapped onto Metal's
   `0..w` depth range rather than the `-w..w` one ToGL's shaders were built
   for, and a depth buffer decides which surface is in front so the BSP's own
   face order does not. Both halves are checked against each other on a
   shipped map rather than a synthetic one: `d1_trainstation_01` parses,
   triangulates into 24,854 triangles over 13,329 shared positions, and fills
   the whole view when rendered from a point the BSP itself reports as open
   space. Which material each surface is drawn with now resolves too, across
   the three lumps that have to agree for a name to exist, and surfaces
   triangulate into runs grouped one per material with texture coordinates
   normalized against each material's own size. Surfaces the world draw never
   shows are left out rather than drawn invisibly: the sky, which the sky box
   draws instead, and the compiler's own `nodraw`, `hint` and `skip`
   annotations. On `d1_trainstation_01` that is 24,005 triangles over 34,993
   vertices in 397 runs across 552 named materials, where the vertex count
   exceeds the untextured one because two surfaces meeting at a corner only
   share it when they also project their texture across it the same way.
   The named content now loads and binds. A material's `.vmt` resolves
   through the same search path the engine builds, a `patch` material is
   followed to what it derives from, `$basetexture` is completed to the one
   lowercase forward-slashed path the archives store, and the `.vtf` it names
   uploads its whole mip chain. The levels are reordered on the way in
   because the file stores them smallest first while a texture numbers them
   largest first, and the block-compressed ones, which is nearly all of them,
   upload exactly as they are stored. A map's own archive is searched ahead
   of the game's, which is what makes the cubemap materials the compiler
   writes per surface position resolvable at all: 256 of
   `d1_trainstation_01`'s 397 materials exist only there. Each material's run
   draws with its own texture bound, out of one shared index buffer, so a
   frame is one draw per material rather than one buffer per material. On
   that map 394 of 397 materials bind a texture, 57.8 MiB in total, and the
   view from a point the BSP reports as open space comes out in 26,296
   distinct shades, which is what separates a sampled frame from a flat fill.
   The three that do not bind are the camera feeds on the map's monitors,
   which the engine renders into rather than loads.
   The world now draws lit rather than flat. A map's baked lighting reads out
   of the lighting lump, preferring the high-range one where the map carries
   it, since a map compiled for high range stores both and the standard-range
   copy is the dimmer fallback the original hardware used. Each lit surface's
   samples are packed into one image with a sample of padding between blocks,
   because binding a texture per surface would cost more than drawing the
   world, and a sample is written the way the engine writes it: divided by
   the overbright so that light brighter than white still fits in a byte, and
   in the screen's gamma rather than linearly. Each vertex carries where it
   falls in that image, from the surface's own lightmap projection, which is a
   separate pair of planes at a far coarser scale than the texture's. Lighting
   a surface costs vertices, because two surfaces that share a corner never
   share a lightmap block, so the corner is stored once per surface rather
   than once per projection: on `d1_trainstation_01` that is 41,440 vertices
   against 34,985 unlit, with 7,783 of the map's 9,033 surfaces lit and packed
   into a 2048x297 image. That the lighting is read, rather than a uniform
   image being sampled at one place, is established by drawing the same
   geometry through a pipeline that ignores it and comparing: across
   twenty-four views from six places in the map the lighting scales the world
   between 0.00 and 0.94, and darkens most of what it touches.
   Only the static light style is read. The later layers belong to lights that
   switch or flicker and need the runtime to animate them, and summing them
   would light a lamp with both its on and off states at once.
   A frame now draws what can be seen from where it stands rather than the
   whole map. Two things decide that, and neither replaces the other. The
   map's visibility set, which the compiler worked out ahead of time, says
   what a wall hides; the view's own six bounding planes say what the screen
   does not hold. The planes come out of the transform being drawn with
   rather than being rebuilt from the eye and lens, because a frustum that
   disagreed with what is drawn would cull what is on screen.
   Culling by the leaves a surface is listed in was tried first, since that
   is the cheap test and the one an engine reaches for, and it was wrong:
   a surface is listed in the leaves it starts in and reaches past them, on
   `d1_trainstation_01` by as much as 1,440 units, and the holes that opened
   were real. Each surface carries its own bounds instead, taken while its
   vertices are already being walked.
   Across twenty-four views from six of the map's own player starts and
   entity positions this draws 4.6 percent of the map's triangles on average
   and 29.2 percent at worst, in 2,215 draws against 9,528. The geometry is
   uploaded once and a frame selects runs out of it, with neighbouring
   visible surfaces in one material coming out as a single run, so nothing
   is rebuilt per frame.
   That it costs nothing visually is established by rendering every view
   twice, once culled and once whole, and requiring no pixel to move by more
   than one step. The bound is on how far a pixel may move rather than on
   how many may move at all, which was measured rather than assumed:
   drawing the same triangles in the same order as more draw calls than
   before already shifts the odd pixel by one, and culling splits a
   material's run wherever the surfaces between were dropped. A hole shows
   whatever was behind the surface, which never lands within one step.
   The map's other 162 brush models are now drawn where they stand. Each is
   a door, lift, breakable or trigger volume the compiler cut out of the
   world, and each is stored about an origin of its own rather than where it
   is seen, so drawing them as stored heaps the map's doors on its origin.
   The entity that owns a model carries the origin and angles that put it
   back, and a model whose entity gives no origin is already in world
   coordinates, which the same transform covers by reading as the identity.
   Whether a model is stored local or in place was read off the map rather
   than assumed: the local origin is not the middle of the brush, since a
   train's sits at one end and a door's at its hinge, so the check is the
   leaf each model's middle lands in. Placed, 158 of the 162 stand in a room
   the visibility set gave a cluster to; stored, 26 do, because the origin a
   local brush sits on is buried in the map's solid.
   Only the position is transformed. Both the texture and the lightmap
   projections are defined against the surface as it was compiled, so a
   placed door carries the texture and the baked light it was built with
   wherever it is put.
   These surfaces are outside what the visibility set describes, because the
   leaves list them where they are stored. Culling them by those leaves
   would be culling them at the origin, so a placed model is tested by the
   leaf it stands in instead: if the view cannot see that cluster, none of
   the model is drawn. Keeping every placed model and leaving it to the view
   frustum also works and is what the first cut did, but it doubled what a
   frame drew, to 9.3 percent of the map's triangles from 4.6. A model whose
   middle lands in solid is still drawn, since that is most often a door
   sitting inside its own frame.
   Two thousand two hundred and four surfaces, a quarter of the map's total,
   were reaching the origin instead of the map before this.
   The props the map is dressed with are now drawn as well, which is the
   first geometry from outside the map file to reach the screen. A static
   prop is a model the map names once in a game-lump dictionary and places
   many times over; `d1_trainstation_01` names 86 and places 298 of them,
   and of those, 82 models and 284 placements are drawn, the rest being
   models the base game does not ship. Each model arrives as three files
   the compiler stamps with one checksum: the `.mdl` holds the body parts,
   meshes and material names, the `.vvd` the vertices, and the `.dx90.vtx`
   the strip groups the triangles are cut from. Joining them is where the
   mistakes live, because each mesh's indices are numbered from that mesh's
   own base within the model's vertices, so losing the base draws a prop
   out of another prop's triangles rather than failing. Placed, this comes
   to 164,447 triangles in 50 material runs, every one of which binds a
   texture the installation ships.
   That the placements land where the map says is checked against each
   model's own reach rather than a fixed distance, since a lamp post's
   geometry stands a long way above the point it is placed at, and 279 of
   the 284 stand more than 512 units from the map's origin, so a dropped
   placement could not pass. The frames are checked for the shape a right
   answer has: props stand about a map rather than covering it, so all four
   views from the player's start hold some, the best drawing 8,852 distinct
   shades over 27 percent of the view. A view filled edge to edge would be
   geometry heaped around the player's feet, and a view of two or three
   shades would be a lost coordinate buffer drawing flat fills, which is
   how the buffer indices being swapped was caught.
   Choosing where to look from turned out to matter as much as what to draw.
   A grid over the map's bounds lands mostly in the void around it and in
   the slivers the compiler leaves inside its walls, which are empty by its
   reckoning but far too small to stand in and enclosed by surfaces facing
   away. A view from inside one sees the backs of the walls, and the
   visibility set correctly reaches almost nothing, so the two frames
   disagreed over thousands of pixels for reasons that had nothing to do
   with culling. The map already says where things are, so the views are
   taken from its own player starts and entity positions.
   Reading the lighting also turned up a defect in the geometry that had
   been there since the faces were first parsed. A face's displacement is
   sixteen bits and the fog volume it bounds is stored immediately after it,
   and the two were being read as one number. Because the compiler writes a
   fog volume of negative one for every ordinary surface, the pair came out
   negative either way, so a real displacement read as none and was drawn
   from edges belonging to geometry that lives in another lump.
   `d1_trainstation_01` has three, which is why nothing looked obviously
   wrong.
4. Add studio models, skinning, facial morphs, decals, particles, water, and
   shadows; the G-Man intro lip-sync gate is an explicit morph regression.
5. Move VGUI/HUD composition and post-processing to the Metal render graph,
   then retire ToGL after campaign-wide visual and stability gates pass.

## Required gates

- Golden captures across representative campaign maps, with separate strict
  compatibility and approved enhanced-quality thresholds.
- GPU validation with no lifetime, synchronization, bounds, or shader errors.
- Correct SDR plus optional HDR/EDR output, Retina scaling, windowed/fullscreen
  transitions, display sleep/wake, and device-loss-style recovery.
- Stable facial morphs, particles, water, transparency, decals, and UI at the
  tested quality presets.
- A measured Apple-silicon frame-time and memory budget plus long-duration
  gameplay soaks before ToGL is removed.
