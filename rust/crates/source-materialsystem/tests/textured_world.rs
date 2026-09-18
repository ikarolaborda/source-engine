//! Draws a real Half-Life 2 map with the textures its materials name.
//!
//! Every crate in this chain is tested on its own: a map triangulates into
//! batches, a material resolves to a texture path, a VTF yields its mip
//! levels, and the device uploads and samples a texture. None of that shows
//! that the chain holds end to end, and each join is somewhere a plausible
//! mistake produces a picture rather than an error. Sampling with the wrong
//! mip order draws a blurred smear, losing the per-vertex texture
//! coordinates draws flat colour, and searching the game's archives without
//! the map's own finds none of the cubemap materials the compiler wrote.
//!
//! So this asserts on what the frame contains: that the world covers the
//! view, that what covers it varies the way sampled textures do rather than
//! being the flat fill a broken sampler would leave, and that the map's
//! baked lighting is what darkens it, by drawing the same view with and
//! without the lightmaps and comparing the two.
//!
//! Shipped content is not redistributable, so this reports that it was
//! skipped when no installation is present.

#![cfg(target_os = "macos")]

use source_bsp::{Bsp, LightmapAtlas, Lightmaps, Materials, Surfaces};
use source_filesystem::{Position, SearchPaths};
use source_materialsystem::MaterialSystem;
use source_render::{
    view_projection, ClearColor, Device, Eye, IndexFormat, Indices, Lens, TextureFormat,
    TriangleList, VertexAttribute, Vertices,
};
use std::collections::HashSet;
use std::path::PathBuf;

const MAP: &str = "d1_trainstation_01";

/// Samples the material's texture at the surface's own coordinates.
///
/// Positions and coordinates arrive as separate buffers because that is how
/// the map stores them, and the fragment stage samples with a linear filter
/// across mip levels, which is what makes a wrong level order visible.
const TEXTURED_SHADER: &str = "
    #include <metal_stdlib>
    using namespace metal;

    struct Surface {
        float4 position [[position]];
        float2 texcoord;
    };

    vertex Surface world_vertex(const device packed_float3 *positions [[buffer(0)]],
                                const device packed_float2 *texcoords [[buffer(2)]],
                                constant float4x4 &view_projection [[buffer(1)]],
                                uint index [[vertex_id]]) {
        Surface out;
        out.position = view_projection * float4(positions[index], 1.0);
        out.texcoord = texcoords[index];
        return out;
    }

    fragment float4 world_fragment(Surface in [[stage_in]],
                                   texture2d<float> base [[texture(0)]]) {
        constexpr sampler tiling(address::repeat, filter::linear, mip_filter::linear);
        return float4(base.sample(tiling, in.texcoord).rgb, 1.0);
    }
";

/// The same, modulated by the surface's baked lighting.
///
/// The lightmap is sampled clamped rather than repeating, because a
/// surface's samples are one block of a packed image and wrapping would
/// reach into an unrelated surface's lighting. The doubling is the
/// overbright the format's samples are stored against, which is what keeps
/// a fully lit surface at its own albedo instead of half of it.
const LIT_SHADER: &str = "
    #include <metal_stdlib>
    using namespace metal;

    struct Surface {
        float4 position [[position]];
        float2 texcoord;
        float2 luxel;
    };

    vertex Surface world_vertex(const device packed_float3 *positions [[buffer(0)]],
                                const device packed_float2 *texcoords [[buffer(2)]],
                                const device packed_float2 *luxels [[buffer(3)]],
                                constant float4x4 &view_projection [[buffer(1)]],
                                uint index [[vertex_id]]) {
        Surface out;
        out.position = view_projection * float4(positions[index], 1.0);
        out.texcoord = texcoords[index];
        out.luxel = luxels[index];
        return out;
    }

    fragment float4 world_fragment(Surface in [[stage_in]],
                                   texture2d<float> base [[texture(0)]],
                                   texture2d<float> lighting [[texture(1)]]) {
        constexpr sampler tiling(address::repeat, filter::linear, mip_filter::linear);
        constexpr sampler packed(address::clamp_to_edge, filter::linear);
        float3 albedo = base.sample(tiling, in.texcoord).rgb;
        float3 light = lighting.sample(packed, in.luxel).rgb * 2.0;
        return float4(albedo * light, 1.0);
    }
";

fn content_root() -> Option<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(content) = std::env::var("SOURCE_HL2_CONTENT_ROOT") {
        roots.push(PathBuf::from(content));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(
            PathBuf::from(home)
                .join("Library/Application Support/Steam/steamapps/common/Half-Life 2"),
        );
    }
    roots
        .into_iter()
        .find(|root| root.join(format!("hl2/maps/{MAP}.bsp")).is_file())
}

fn as_bytes<T>(values: &[T]) -> &[u8] {
    // SAFETY: the callers pass arrays of `f32` and `u32`, which have no
    // padding and no invalid bit patterns, so their bytes are readable.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

/// Points inside the map's open space, as in the geometry test: a leaf with
/// a cluster is one the visibility set covers, which is where a player can
/// stand.
///
/// Several are wanted rather than one, each from a different leaf, because
/// one point says nothing about a map: the first open space found in a
/// corner of the bounds is as likely to be a sealed dark shaft as the
/// station concourse, and a claim about how the world draws has to be made
/// from more of it than that.
/// Where in the map to look from, taken from the map's own entities.
///
/// A grid over the map's bounds mostly lands in the void around it or in
/// the slivers the compiler leaves inside its walls. The map itself already
/// says where things are: the places it starts the player, and the
/// hundreds of props, lights and triggers it positions. Those are in the
/// rooms, which is where a view is worth rendering from.
///
/// The player's starts come first, since they are the only positions the
/// map promises are standable, and one per leaf is enough.
fn places_in_the_map(world: &source_bsp::World, entities: &source_bsp::Entities) -> Vec<[f32; 3]> {
    // A player is seventy-two units tall, so a leaf smaller than that in
    // any direction is one of the slivers rather than a room.
    const ROOM: i16 = 72;
    // Entity origins sit at the foot of whatever they place. A view from
    // the floor is mostly floor, so it is raised to about eye height.
    const EYE: f32 = 48.0;

    let starts = entities
        .by_classname("info_player_start")
        .filter_map(source_bsp::Entity::origin);
    let rest = entities
        .iter()
        .filter(|entity| entity.brush_model().is_none())
        .filter_map(source_bsp::Entity::origin);

    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for origin in starts.chain(rest) {
        let point = [origin[0], origin[1], origin[2] + EYE];
        let Ok(index) = world.point_leaf(point) else {
            continue;
        };
        let leaf = &world.leaves()[index];
        if leaf.cluster < 0 || (0..3).any(|axis| leaf.maxs[axis] - leaf.mins[axis] < ROOM) {
            continue;
        }
        if seen.insert(index) {
            found.push(point);
        }
    }
    found
}

/// Writes a readback as an uncompressed bitmap, which needs no encoder and
/// which every image viewer reads.
fn write_bitmap(path: &std::path::Path, frame: &source_render::Readback, size: u32) {
    let stride = (size * 4) as usize;
    let pixel_bytes = stride * size as usize;
    let mut out = Vec::with_capacity(54 + pixel_bytes);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(size as i32).to_le_bytes());
    // A negative height means the rows run top down, as the readback does.
    out.extend_from_slice(&(-(size as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in 0..size {
        for x in 0..size {
            let pixel = frame.pixel(x, y).unwrap_or([0, 0, 0, 255]);
            out.extend_from_slice(&pixel);
        }
    }
    if let Err(error) = std::fs::write(path, &out) {
        eprintln!("could not write {}: {error}", path.display());
    }
}

#[test]
fn textures_the_world_of_a_shipped_map_with_the_materials_it_names() {
    let Some(root) = content_root() else {
        eprintln!("skipped: no installed Half-Life 2 content to read a map from");
        return;
    };

    // The search path the engine builds for a loaded map: the game's loose
    // files and archives, with the map's own archive ahead of them.
    let mut paths = SearchPaths::new();
    paths
        .mount_directory(root.join("hl2"), "GAME", Position::Tail)
        .expect("the game directory mounts");
    for name in ["hl2_misc", "hl2_textures", "hl2_pak"] {
        let archive = root.join(format!("hl2/{name}_dir.vpk"));
        if archive.is_file() {
            paths
                .mount_vpk(&archive, "GAME", Position::Tail)
                .expect("a shipped archive mounts");
        }
    }

    let bytes = paths
        .read(&format!("maps/{MAP}.bsp"), None)
        .expect("the map reads");
    let bsp = Bsp::parse(&bytes).expect("a shipped map parses");
    paths
        .mount_pak(bsp.pakfile(), MAP, "GAME", Position::Head)
        .expect("the map's own archive mounts");

    let surfaces = Surfaces::parse(&bsp).expect("a shipped map has world surfaces");
    let materials = Materials::parse(&bsp).expect("a shipped map names its materials");
    let lighting = Lightmaps::parse(&bsp).expect("a shipped map has baked lighting");
    assert!(
        !lighting.is_empty(),
        "a shipped map's world is lit, so its lighting lump is not empty"
    );
    let world = source_bsp::World::parse(&bsp).expect("a shipped map has a world");
    let entities = source_bsp::Entities::parse(&bsp).expect("a shipped map places entities");
    // The world model is drawn where it is stored; the map's other models
    // are its doors, lifts and trigger volumes, each cut out and stored
    // about an origin of its own, so each is drawn where the entity that
    // places it says. Triggers fall out on their own, because a surface
    // whose texinfo is not drawn never reaches the geometry.
    let drawn = world.world_faces();
    let placements = entities.brush_placements();
    let mut groups: Vec<(source_bsp::Placement, Vec<usize>)> =
        vec![(source_bsp::Placement::IDENTITY, drawn.clone().collect())];
    for (model, placement) in &placements {
        groups.push((*placement, world.models()[*model].faces().collect()));
    }
    let lit_faces: Vec<usize> = groups
        .iter()
        .flat_map(|(_, faces)| faces.iter().copied())
        .collect();
    let atlas = LightmapAtlas::pack(&surfaces, &materials, &lighting, lit_faces)
        .expect("a shipped map's lightmaps pack");
    let geometry = surfaces
        .triangulate_placed(&materials, Some(&atlas), groups)
        .expect("a shipped map triangulates");
    assert!(
        drawn.end < surfaces.faces().len(),
        "this map has brush models beyond its world, which its entities place"
    );
    let placed_runs = geometry.runs.iter().filter(|run| run.group != 0).count();
    assert!(
        placed_runs > 500,
        "the map's doors, lifts and breakables are drawn too, got {placed_runs} runs"
    );
    assert_eq!(
        geometry.lightmap_coords.len(),
        geometry.positions.len(),
        "every vertex carries where it falls in the packed lighting"
    );

    let device = Device::open().expect("Apple silicon has a Metal device");
    let flat = device.compile_library(TEXTURED_SHADER).expect("compiled");
    let flat = device
        .create_depth_pipeline(&flat, "world_vertex", "world_fragment")
        .expect("built");
    let lit = device.compile_library(LIT_SHADER).expect("compiled");
    let lit = device
        .create_depth_pipeline(&lit, "world_vertex", "world_fragment")
        .expect("built");
    let packed_lighting = device
        .create_texture(
            atlas.width(),
            atlas.height(),
            TextureFormat::Bgra8Unorm,
            atlas.pixels(),
        )
        .expect("the packed lighting uploads");

    // Every material the map's batches name, loaded once each.
    let mut system = MaterialSystem::new();
    let mut bound = 0usize;
    let mut unbound: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut names = HashSet::new();
    for batch in &geometry.batches {
        let name = materials
            .name(batch.texdata)
            .expect("a batch names a material");
        if !names.insert(name.to_owned()) {
            continue;
        }
        match system.bind(&device, &paths, name) {
            Ok(binding) => match &binding.unbound {
                None => bound += 1,
                Some(reason) => unbound.push(reason.to_string()),
            },
            Err(error) => failed.push(error.to_string()),
        }
    }

    assert!(
        failed.is_empty(),
        "{} of {} materials could not be loaded at all, first few: {:?}",
        failed.len(),
        names.len(),
        &failed[..failed.len().min(5)]
    );
    // What remains unbound is the handful the engine renders itself rather
    // than loads, chiefly the camera feeds on the monitors in this map.
    assert!(
        bound * 20 > names.len() * 19,
        "nearly every material bound a texture, {bound} of {} did, unbound: {:?}",
        names.len(),
        &unbound[..unbound.len().min(10)]
    );

    let positions = device
        .create_buffer(as_bytes(&geometry.positions))
        .expect("positions upload");
    let texcoords = device
        .create_buffer(as_bytes(&geometry.texcoords))
        .expect("texture coordinates upload");
    let luxels = device
        .create_buffer(as_bytes(&geometry.lightmap_coords))
        .expect("lightmap coordinates upload");
    let indices = device
        .create_buffer(as_bytes(&geometry.indices))
        .expect("indices upload");

    let mut low = [f32::MAX; 3];
    let mut high = [f32::MIN; 3];
    for position in &geometry.positions {
        for axis in 0..3 {
            low[axis] = low[axis].min(position[axis]);
            high[axis] = high[axis].max(position[axis]);
        }
    }
    let entities = source_bsp::Entities::parse(&bsp).expect("a shipped map places entities");
    let standing = places_in_the_map(&world, &entities);
    assert!(
        standing.len() >= 8,
        "a shipped map has many places to stand, found {} among {} entities",
        standing.len(),
        entities.len()
    );

    let lens = Lens {
        horizontal_fov: 90.0,
        aspect: 1.0,
        near: 1.0,
        far: (high[0] - low[0]).max(high[1] - low[1]).max(1000.0) * 2.0,
    };
    let black = ClearColor {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alpha: 1.0,
    };

    // Larger when a frame is being written out to look at, since the
    // assertions below hold at either size.
    let size: u32 = match std::env::var("SOURCE_RENDER_DUMP_SIZE") {
        Ok(value) => value.parse().unwrap_or(128),
        Err(_) => 128,
    };
    let size_squared = (size * size) as f32;

    let world_buffers = WorldBuffers {
        geometry: &geometry,
        materials: &materials,
        system: &system,
        positions: &positions,
        texcoords: &texcoords,
        luxels: &luxels,
        indices: &indices,
    };

    // Sampled from several places in the map rather than one, so the
    // claims below are about how the map draws rather than about one corner
    // of it.
    let mut best: Option<Shot> = None;
    let mut ratios: Vec<f32> = Vec::new();
    let mut compared = 0usize;
    let mut darkened = 0usize;
    let mut views = 0usize;
    let mut culling = Culling::default();
    for position in standing.iter().take(VIEWPOINTS) {
        for yaw in [0.0, 90.0, 180.0, 270.0] {
            let eye = Eye {
                position: *position,
                angles: [0.0, yaw, 0.0],
            };
            let transform = view_projection(eye, lens).expect("a square lens");

            let everything = world_buffers.geometry.batches.clone();
            let draws = world_buffers.draws(&lit, Some(&packed_lighting), &transform, &everything);
            assert!(
                draws.len() > 300,
                "the map draws once per material, got {} draws",
                draws.len()
            );
            let lit_frame = device
                .render_offscreen(size, size, black, &draws)
                .expect("the map's lit world renders");

            // The same frame with the map's own visibility set and this
            // view's bounding planes deciding what is drawn. The map holds
            // far more of the world than any one place in it can see, and
            // the compiler worked out ahead of time which parts reach
            // which. Drawing the rest is work no pixel depends on.
            let visible = source_bsp::VisibleFaces::select(&world, &surfaces, *position)
                .expect("a point standing inside the map has a visibility row")
                .placing(&world, &placements);
            let frustum = source_bsp::Frustum::new(transform.frustum_planes());
            let culled_batches = world_buffers
                .geometry
                .visible_batches(&visible, Some(&frustum));
            let culled_frame = device
                .render_offscreen(
                    size,
                    size,
                    black,
                    &world_buffers.draws(&lit, Some(&packed_lighting), &transform, &culled_batches),
                )
                .expect("the map's visible world renders");
            culling.record(&visible, &culled_batches, &everything, &geometry);

            // The same geometry through the pipeline that ignores the
            // lighting. Lightmap coordinates that all pointed at one
            // sample, or a packed image that came out uniform, would still
            // draw a textured world; what shows the lighting is read is
            // that it changes the picture, and changes it by different
            // amounts in different places.
            let unlit_frame = device
                .render_offscreen(
                    size,
                    size,
                    black,
                    &world_buffers.draws(&flat, None, &transform, &everything),
                )
                .expect("the map's unlit world renders");
            views += 1;

            // Culling is only worth anything if it does not change the
            // picture. Anything the visibility set or the view drops is
            // either behind world geometry the depth buffer would have
            // settled anyway or off the edge of the screen, so the frame
            // that skips it has to come out the same.
            //
            // The bound is on how far a pixel may move, not on how many
            // may move at all, and that is deliberate. Drawing the same
            // triangles in the same order as more draw calls than before
            // already shifts the odd pixel by a step, which culling does
            // by its nature: it splits a material's run where the
            // surfaces between were dropped. What culling must not do is
            // leave a hole, and a hole shows whatever was behind the
            // surface: a different material at a different distance under
            // different light, which never lands within one step of it.
            let mut differing = 0usize;
            let mut furthest = 0i32;
            for y in 0..size {
                for x in 0..size {
                    let whole = lit_frame.pixel(x, y).expect("the frame is square");
                    let part = culled_frame.pixel(x, y).expect("the frame is square");
                    if whole == part {
                        continue;
                    }
                    differing += 1;
                    for channel in 0..4 {
                        let moved = i32::from(whole[channel]) - i32::from(part[channel]);
                        furthest = furthest.max(moved.abs());
                    }
                }
            }
            assert!(
                furthest <= 1,
                "culling moved a pixel by {furthest} looking {yaw:.0} from {position:?}, so it dropped something on screen"
            );
            assert!(
                differing * 1000 < (size * size) as usize,
                "culling left all but a handful of the frame untouched, {differing} of {} pixels moved",
                size * size
            );

            let mut covered = 0usize;
            let mut shades = HashSet::new();
            for y in 0..size {
                for x in 0..size {
                    let unlit = unlit_frame.pixel(x, y).expect("the frame is square");
                    if unlit[..3] == [0, 0, 0] {
                        continue;
                    }
                    covered += 1;
                    shades.insert([unlit[0], unlit[1], unlit[2]]);

                    let before = luminance(unlit);
                    if before < 16.0 {
                        // Somewhere the world's own texture is nearly
                        // black, where a ratio says nothing about light.
                        continue;
                    }
                    let after = luminance(lit_frame.pixel(x, y).expect("the frame is square"));
                    compared += 1;
                    if after + 1.0 < before {
                        darkened += 1;
                    }
                    ratios.push(after / before);
                }
            }

            let coverage = covered as f32 / size_squared;
            if best.as_ref().is_none_or(|shot| coverage > shot.coverage) {
                best = Some(Shot {
                    coverage,
                    shades: shades.len(),
                    position: *position,
                    yaw,
                    lit: lit_frame,
                    unlit: unlit_frame,
                });
            }
        }
    }

    let best = best.expect("a view was rendered");
    // A frame is easier to judge by eye than by pixel counts, so the view
    // that sees most of the world is written out when asked for.
    if let Some(path) = std::env::var_os("SOURCE_RENDER_DUMP") {
        write_bitmap(path.as_ref(), &best.lit, size);
    }
    if let Some(path) = std::env::var_os("SOURCE_RENDER_DUMP_UNLIT") {
        write_bitmap(path.as_ref(), &best.unlit, size);
    }

    eprintln!(
        "{} materials, {bound} textured, {:.1} MiB uploaded; lighting packed into {}x{} for {} surfaces; {} triangles in {} batches; over {views} views the best filled {:.0}% of the view in {} shades",
        names.len(),
        system.uploaded_bytes() as f32 / (1024.0 * 1024.0),
        atlas.width(),
        atlas.height(),
        atlas.len(),
        geometry.triangle_count(),
        geometry.batches.len(),
        best.coverage * 100.0,
        best.shades,
    );

    eprintln!(
        "over {} views culling drew {:.1}% of the map's triangles on average and {:.1}% at worst, {} of {} surfaces, in {} draws rather than {}",
        culling.views,
        culling.share() * 100.0,
        culling.worst_share * 100.0,
        culling.drawn_faces,
        culling.whole_faces,
        culling.draws,
        culling.whole_draws,
    );
    // The whole point of reading the visibility set is that a frame stops
    // paying for the parts of the map it cannot see. A map this size is
    // mostly out of view from anywhere in it, so the saving is not
    // marginal, and a selection that quietly kept everything would still
    // have passed the comparison above.
    assert!(
        culling.worst_share < 0.5,
        "no view drew even half the map, the most any drew was {:.1}%",
        culling.worst_share * 100.0
    );
    assert!(
        culling.share() < 0.2,
        "a view draws a small part of the map, averaged {:.1}%",
        culling.share() * 100.0
    );
    assert!(
        culling.drawn_faces > 0 && culling.draws > 0,
        "and it still draws something"
    );

    assert!(
        best.coverage > 0.5,
        "standing inside the map, the world fills most of the view, got {:.0}%",
        best.coverage * 100.0
    );
    // A sampler given the wrong coordinates, the wrong mip order or no
    // texture at all leaves large flat areas. Real surfaces drawn with real
    // textures vary continuously, so the count of distinct colours is what
    // separates a textured frame from a filled one. This is measured
    // without the lighting, because a genuinely dark corner of the map
    // would crush the count whatever the textures did.
    assert!(
        best.shades > 500,
        "the view is textured rather than flat, got {} distinct shades",
        best.shades
    );

    assert!(
        compared > views * 1000,
        "enough of the world is drawn across the views to compare, got {compared} pixels"
    );
    assert!(
        darkened * 2 > compared,
        "the baked lighting darkens the world where it is not fully lit, {darkened} of {compared} pixels"
    );

    ratios.sort_by(f32::total_cmp);
    let at = |fraction: f32| ratios[((ratios.len() - 1) as f32 * fraction) as usize];
    let (dim, bright) = (at(0.02), at(0.98));
    eprintln!(
        "across {views} views the lighting scales the world between {dim:.2} and {bright:.2}; the best looks {:.0} from {:?}",
        best.yaw, best.position,
    );
    // A uniform lightmap, or coordinates that sample one place, would scale
    // every surface by the same amount. Real baked lighting puts lit and
    // shadowed surfaces in the same map, so the range across it is wide.
    assert!(
        bright - dim > 0.5,
        "the lighting varies across the map rather than scaling it evenly, {dim:.2} to {bright:.2}"
    );
    assert!(
        dim < 0.5,
        "some of the map is genuinely shadowed, dimmest fiftieth is {dim:.2}"
    );
    assert!(
        bright > 0.5,
        "some of the map is genuinely lit, brightest fiftieth is {bright:.2}"
    );
}

/// What culling removed, summed over the views, so the claim that it saves
/// work is made about the map rather than about one lucky corner of it.
#[derive(Default)]
struct Culling {
    views: usize,
    drawn_indices: usize,
    whole_indices: usize,
    drawn_faces: usize,
    whole_faces: usize,
    draws: usize,
    whole_draws: usize,
    worst_share: f32,
}

impl Culling {
    fn record(
        &mut self,
        visible: &source_bsp::VisibleFaces,
        culled: &[source_bsp::Batch],
        everything: &[source_bsp::Batch],
        geometry: &source_bsp::TexturedGeometry,
    ) {
        let drawn: usize = culled.iter().map(|batch| batch.index_count).sum();
        let whole: usize = everything.iter().map(|batch| batch.index_count).sum();
        assert_eq!(whole, geometry.indices.len(), "the map draws all of itself");
        self.views += 1;
        self.drawn_indices += drawn;
        self.whole_indices += whole;
        self.drawn_faces += geometry
            .runs
            .iter()
            .filter(|run| visible.contains(run.face))
            .count();
        self.whole_faces += geometry.runs.len();
        self.draws += culled.len();
        self.whole_draws += everything.len();
        self.worst_share = self.worst_share.max(drawn as f32 / whole as f32);
    }

    /// The share of the map a view drew, averaged over the views.
    fn share(&self) -> f32 {
        self.drawn_indices as f32 / self.whole_indices as f32
    }
}

/// How many places in the map are looked at. Each is rendered once per
/// heading, with and without the lighting.
const VIEWPOINTS: usize = 6;

/// One rendered view kept for comparison, with where it was taken from.
struct Shot {
    coverage: f32,
    shades: usize,
    position: [f32; 3],
    yaw: f32,
    lit: source_render::Readback,
    unlit: source_render::Readback,
}

/// Perceived brightness, so that lighting a surface is measured the way it
/// is seen rather than by summing channels the eye weighs differently.
fn luminance(pixel: [u8; 4]) -> f32 {
    0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2])
}

/// The map's world as the GPU holds it, so that the same geometry can be
/// drawn through either pipeline and the frames differ only in the lighting.
struct WorldBuffers<'a> {
    geometry: &'a source_bsp::TexturedGeometry,
    materials: &'a Materials,
    system: &'a MaterialSystem,
    positions: &'a source_render::Buffer,
    texcoords: &'a source_render::Buffer,
    luxels: &'a source_render::Buffer,
    indices: &'a source_render::Buffer,
}

impl<'a> WorldBuffers<'a> {
    /// One draw per run of the shared index buffer, each with the texture
    /// its material names bound.
    ///
    /// The runs are passed in rather than taken from the geometry, because
    /// what a frame draws is a choice a frame makes: the whole map, or the
    /// part of it the view can reach.
    fn draws(
        &self,
        pipeline: &'a source_render::Pipeline,
        lighting: Option<&'a source_render::Texture>,
        transform: &'a source_render::Matrix,
        batches: &[source_bsp::Batch],
    ) -> Vec<TriangleList<'a>> {
        batches
            .iter()
            .filter_map(|batch| {
                let name = self.materials.name(batch.texdata)?;
                let texture = self.system.get(name)?.texture.as_ref()?;
                let draw = TriangleList::new(
                    pipeline,
                    Vertices::Buffered {
                        buffer: self.positions,
                        count: self.geometry.positions.len(),
                        stride: std::mem::size_of::<[f32; 3]>(),
                    },
                )
                .with_texture(texture)
                .with_coordinates(VertexAttribute::packed::<[f32; 2]>(self.texcoords))
                .with_indices(Indices {
                    buffer: self.indices,
                    count: batch.index_count,
                    format: IndexFormat::Uint32,
                    first: batch.first_index,
                })
                .with_uniforms(transform.as_bytes());
                Some(match lighting {
                    Some(lighting) => draw
                        .with_lightmap(lighting, VertexAttribute::packed::<[f32; 2]>(self.luxels)),
                    None => draw,
                })
            })
            .collect()
    }
}
