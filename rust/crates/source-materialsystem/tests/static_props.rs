//! Draws the props a real Half-Life 2 map is dressed with.
//!
//! A map's world is its walls and floors. Everything standing on them —
//! crates, lamps, railings, signs — is a static prop: a model file the map
//! names once in a dictionary and places hundreds of times. Drawing them
//! joins four formats that are each tested on their own, and every join is
//! somewhere a plausible mistake yields geometry rather than an error.
//! Losing the per-mesh vertex base draws triangles pulled from the wrong
//! parts of the model; losing the placement draws every prop stacked on the
//! map's origin; taking the prop lump's stride wrong reads each prop from
//! the middle of its neighbour.
//!
//! So this asserts on where the geometry ends up and on what the frame
//! contains: that each prop's vertices land around the origin the map gave
//! it, that the props resolve to materials the installation ships, and that
//! drawing them puts something textured on screen.
//!
//! Shipped content is not redistributable, so this reports that it was
//! skipped when no installation is present.
use source_bsp::{Bsp, StaticProps};
use source_filesystem::{Position, SearchPaths};
use source_materialsystem::MaterialSystem;
use source_render::{
    view_projection, ClearColor, Device, Eye, IndexFormat, Indices, Lens, TriangleList,
    VertexAttribute, Vertices,
};

const MAP: &str = "d1_trainstation_01";

const PROP_SHADER: &str = "
    #include <metal_stdlib>
    using namespace metal;

    struct Surface {
        float4 position [[position]];
        float2 texcoord;
    };

    vertex Surface prop_vertex(const device packed_float3 *positions [[buffer(0)]],
                               const device packed_float2 *texcoords [[buffer(2)]],
                               constant float4x4 &view_projection [[buffer(1)]],
                               uint index [[vertex_id]]) {
        Surface out;
        out.position = view_projection * float4(positions[index], 1.0);
        out.texcoord = texcoords[index];
        return out;
    }

    fragment float4 prop_fragment(Surface in [[stage_in]],
                                  texture2d<float> base [[texture(0)]]) {
        constexpr sampler tiling(address::repeat, filter::linear, mip_filter::linear);
        return float4(base.sample(tiling, in.texcoord).rgb, 1.0);
    }
";

fn content_root() -> Option<std::path::PathBuf> {
    let root = std::env::var_os("SOURCE_HL2_CONTENT_ROOT")?;
    let root = std::path::PathBuf::from(root);
    root.join("hl2").is_dir().then_some(root)
}

/// One model's geometry, in the model's own space, ready to be placed.
struct Model {
    positions: Vec<[f32; 3]>,
    texcoords: Vec<[f32; 2]>,
    /// One run of indices per material the model draws with.
    runs: Vec<(String, Vec<u32>)>,
}

fn load_model(paths: &SearchPaths, name: &str) -> Option<Model> {
    let mdl_bytes = paths.read(name, None).ok()?;
    let vvd_bytes = paths.read(&name.replace(".mdl", ".vvd"), None).ok()?;
    let vtx_bytes = paths.read(&name.replace(".mdl", ".dx90.vtx"), None).ok()?;

    let mdl = source_studio::Mdl::parse(&mdl_bytes).ok()?;
    let vvd = source_studio::Vvd::parse(&vvd_bytes).ok()?;
    let vtx = source_studio::Vtx::parse(&vtx_bytes).ok()?;
    // The three files are one model split across three, and the compiler
    // stamps each with the same checksum so a mismatched set is caught
    // before it is drawn as nonsense.
    assert_eq!(mdl.checksum, vvd.checksum, "{name} ships matching vertices");
    assert_eq!(
        mdl.checksum, vtx.checksum,
        "{name} ships matching triangles"
    );

    let vertices = vvd.vertices(&vvd_bytes, 0).ok()?;
    let meshes = source_studio::triangles(&mdl, &mdl_bytes, &vtx, &vtx_bytes, 0).ok()?;
    let (materials, directories) = mdl.materials(&mdl_bytes).ok()?;

    let mut runs: Vec<(String, Vec<u32>)> = Vec::new();
    for mesh in meshes {
        let material = materials.get(mesh.material)?;
        let resolved = directories.iter().find_map(|directory| {
            let candidate = format!("{directory}{material}");
            paths
                .read(&format!("materials/{candidate}.vmt"), None)
                .is_ok()
                .then_some(candidate)
        })?;
        match runs.iter_mut().find(|(name, _)| *name == resolved) {
            Some((_, indices)) => indices.extend_from_slice(&mesh.indices),
            None => runs.push((resolved, mesh.indices)),
        }
    }

    Some(Model {
        positions: vertices.iter().map(|vertex| vertex.position).collect(),
        texcoords: vertices.iter().map(|vertex| vertex.texcoord).collect(),
        runs,
    })
}

#[test]
fn draws_the_static_props_a_shipped_map_places() {
    let Some(root) = content_root() else {
        eprintln!("skipped: no installed Half-Life 2 content to read props from");
        return;
    };

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
    let map_bytes = paths
        .read(&format!("maps/{MAP}.bsp"), None)
        .expect("the map reads");
    let bsp = Bsp::parse(&map_bytes).expect("a shipped map parses");
    paths
        .mount_pak(bsp.pakfile(), MAP, "GAME", Position::Head)
        .expect("the map's own archive mounts");

    let props = StaticProps::parse(&bsp).expect("a shipped map's props parse");
    assert!(!props.is_empty(), "a campaign map is dressed with props");

    // Loaded once each and placed as many times as the map places them,
    // which is the reason the dictionary exists.
    let models: Vec<Option<Model>> = props
        .names()
        .iter()
        .map(|name| load_model(&paths, name))
        .collect();
    let loaded = models.iter().filter(|model| model.is_some()).count();
    assert!(
        loaded * 10 > props.names().len() * 9,
        "the installation ships the models its own map names, {loaded} of {} loaded",
        props.names().len()
    );

    // Every prop's geometry, transformed out of the model's own space into
    // the map's, in one buffer with one run per material.
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut texcoords: Vec<[f32; 2]> = Vec::new();
    let mut runs: Vec<(String, Vec<u32>)> = Vec::new();
    let mut placed = 0usize;
    let mut off_origin = 0usize;
    for prop in props.props() {
        let Some(model) = models.get(prop.model).and_then(Option::as_ref) else {
            continue;
        };
        let placement = StaticProps::placement(prop);
        let base = u32::try_from(positions.len()).expect("a map's props fit an index");
        let mut middle = [0.0f64; 3];
        for (position, texcoord) in model.positions.iter().zip(&model.texcoords) {
            let world = placement.apply(*position);
            for axis in 0..3 {
                middle[axis] += f64::from(world[axis]);
            }
            positions.push(world);
            texcoords.push(*texcoord);
        }

        // Placing a prop means putting its own space where the map says,
        // so its vertices come out around the origin the map gave it,
        // within the model's own size. A placement that was dropped leaves
        // them around zero instead, and this map's props stand thousands of
        // units from there. The model's own reach is the bound rather than
        // a fixed distance, because a lamppost's geometry stands a long way
        // above the point the map places it at.
        let count = model.positions.len() as f64;
        if count > 0.0 {
            let middle: [f32; 3] = std::array::from_fn(|axis| (middle[axis] / count) as f32);
            let reach = model
                .positions
                .iter()
                .flat_map(|position| position.iter())
                .fold(1.0f32, |reach, value| reach.max(value.abs()));
            let strayed = (0..3)
                .map(|axis| (middle[axis] - prop.origin[axis]).abs())
                .fold(0.0f32, f32::max);
            assert!(
                strayed <= reach,
                "a prop placed at {:?} has its geometry centred at {middle:?}, \
                 {strayed} away from a model that reaches {reach}",
                prop.origin
            );
            off_origin += usize::from(prop.origin.iter().any(|value| value.abs() > 512.0));
        }

        for (material, indices) in &model.runs {
            let shifted: Vec<u32> = indices.iter().map(|index| index + base).collect();
            match runs.iter_mut().find(|(name, _)| name == material) {
                Some((_, run)) => run.extend_from_slice(&shifted),
                None => runs.push((material.clone(), shifted)),
            }
        }
        placed += 1;
    }

    assert!(placed > 200, "the map places its props, got {placed}");
    assert!(
        off_origin * 10 > placed * 9,
        "the map stands its props away from its origin, only {off_origin} of {placed} do"
    );
    let triangles = runs.iter().map(|(_, run)| run.len() / 3).sum::<usize>();
    assert!(
        triangles > 10_000,
        "a map's worth of props is real geometry, got {triangles} triangles"
    );

    let device = Device::open().expect("Apple silicon has a Metal device");
    let library = device.compile_library(PROP_SHADER).expect("compiled");
    let pipeline = device
        .create_depth_pipeline(&library, "prop_vertex", "prop_fragment")
        .expect("built");

    let mut system = MaterialSystem::new();
    let mut bound = 0usize;
    for (material, _) in &runs {
        if system
            .bind(&device, &paths, material)
            .is_ok_and(source_materialsystem::Binding::is_bound)
        {
            bound += 1;
        }
    }
    assert!(
        bound * 10 > runs.len() * 9,
        "the props' materials bind textures, {bound} of {} did",
        runs.len()
    );

    let position_buffer = device
        .create_buffer(as_bytes(&positions))
        .expect("positions upload");
    let texcoord_buffer = device
        .create_buffer(as_bytes(&texcoords))
        .expect("coordinates upload");

    // Standing where the map starts the player and turning around, because
    // a viewpoint chosen by hand can be one with nothing in front of it.
    let entities = source_bsp::Entities::parse(&bsp).expect("the map places entities");
    let start = entities
        .by_classname("info_player_start")
        .find_map(|entity| entity.origin())
        .expect("a campaign map starts the player somewhere");
    let size = 256;
    let lens = Lens {
        horizontal_fov: 90.0,
        aspect: 1.0,
        near: 4.0,
        far: 16384.0,
    };

    let mut drew = 0usize;
    let mut best_shades = 0usize;
    let mut covered: Vec<f32> = Vec::new();
    for yaw in [0.0, 90.0, 180.0, 270.0] {
        let eye = Eye {
            position: [start[0], start[1], start[2] + 48.0],
            angles: [0.0, yaw, 0.0],
        };
        let transform = view_projection(eye, lens).expect("a square lens");

        let mut buffers = Vec::new();
        for (material, indices) in &runs {
            let Some(texture) = system
                .get(material)
                .and_then(|bound| bound.texture.as_ref())
            else {
                continue;
            };
            let buffer = device
                .create_buffer(as_bytes(indices))
                .expect("indices upload");
            buffers.push((texture, buffer, indices.len()));
        }
        let draws: Vec<TriangleList<'_>> = buffers
            .iter()
            .map(|(texture, buffer, count)| {
                TriangleList::new(
                    &pipeline,
                    Vertices::Buffered {
                        buffer: &position_buffer,
                        count: positions.len(),
                        stride: std::mem::size_of::<[f32; 3]>(),
                    },
                )
                .with_texture(texture)
                .with_coordinates(VertexAttribute::packed::<[f32; 2]>(&texcoord_buffer))
                .with_indices(Indices {
                    buffer,
                    count: *count,
                    format: IndexFormat::Uint32,
                    first: 0,
                })
                .with_uniforms(transform.as_bytes())
            })
            .collect();

        let black = ClearColor {
            red: 0.0,
            green: 0.0,
            blue: 0.0,
            alpha: 1.0,
        };
        let frame = device
            .render_offscreen(size, size, black, &draws)
            .expect("the map's props render");

        // Props are scattered rather than covering, so this asks that they
        // are there and textured rather than that they fill the view: a
        // handful of distinct shades would be a flat fill, which is what a
        // lost coordinate buffer draws.
        let drawn = frame
            .pixels
            .chunks_exact(4)
            .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
            .count();
        let shades: std::collections::HashSet<[u8; 3]> = frame
            .pixels
            .chunks_exact(4)
            .filter(|pixel| pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0)
            .map(|pixel| [pixel[0], pixel[1], pixel[2]])
            .collect();
        // Props stand about the map rather than covering it, so a view
        // holding a few of them is what a right answer looks like. What
        // would be wrong is a view holding none, or one filled edge to
        // edge, which is what geometry left at the origin around the
        // player's feet would draw.
        // A picture of what was drawn, for a person to look at, where one
        // is asked for.
        if let Some(directory) = std::env::var_os("SOURCE_PROP_SHOT_DIR") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).expect("a place to write shots");
            let mut ppm = format!("P6\n{size} {size}\n255\n").into_bytes();
            ppm.extend(frame.pixels.chunks_exact(4).flat_map(|pixel| &pixel[..3]));
            std::fs::write(directory.join(format!("props-yaw{yaw:.0}.ppm")), ppm)
                .expect("a shot writes");
        }
        if drawn > 0 {
            drew += 1;
            covered.push(drawn as f32 / (size * size) as f32);
        }
        best_shades = best_shades.max(shades.len());
    }

    assert!(
        drew > 0,
        "the props the map places are visible from where it starts the player"
    );
    assert!(
        best_shades > 200,
        "the props draw their own textures rather than a flat fill, got {best_shades} shades"
    );
    let most = covered.iter().fold(0.0f32, |most, share| most.max(*share));
    assert!(
        most < 0.95,
        "the props stand about the map rather than filling the view, which is what \
         geometry left on the origin under the player would draw, and one view was \
         {:.0}% covered",
        most * 100.0
    );

    eprintln!(
        "{placed} props over {loaded} models, {triangles} triangles in {} material runs, \
         {bound} bound; {drew} of four views held props, the best drawing {best_shades} \
         shades over {:.0}% of the view",
        runs.len(),
        most * 100.0
    );
}

/// A slice of a plain type as the bytes a buffer takes.
///
/// Safe because the types used here are arrays of floats and integers,
/// which have no padding and no invalid bit patterns, and the result is
/// only ever read as bytes.
fn as_bytes<T: Copy>(values: &[T]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}
