//! Renders the world surfaces of a real Half-Life 2 map through the Metal
//! device.
//!
//! The unit tests in each crate check their own half: that a BSP triangulates
//! into shared indexed corners, and that indexed world-space positions reach
//! the screen through a camera transform. Neither proves the two halves
//! agree about vertex layout, index width, winding or scale, which is what a
//! real map exercises and a synthetic one cannot.
//!
//! Shipped content is not redistributable, so this locates an installed copy
//! and reports that it was skipped when there is none. The GPU tests that do
//! run everywhere live in the crate's own test module.

#![cfg(target_os = "macos")]

use source_bsp::{Bsp, Surfaces};
use source_render::{
    view_projection, ClearColor, Device, Eye, IndexFormat, Indices, Lens, TriangleList, Vertices,
};
use std::path::PathBuf;

const WORLD_SHADER: &str = "
    #include <metal_stdlib>
    using namespace metal;
    vertex float4 world_vertex(const device packed_float3 *positions [[buffer(0)]],
                               constant float4x4 &view_projection [[buffer(1)]],
                               uint index [[vertex_id]]) {
        return view_projection * float4(positions[index], 1.0);
    }
    fragment float4 world_fragment() {
        return float4(0.0, 1.0, 0.0, 1.0);
    }
";

/// The first map of the campaign, from the staged runtime tree or an
/// installed copy.
fn find_map() -> Option<PathBuf> {
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
        .map(|root| root.join("hl2/maps/d1_trainstation_01.bsp"))
        .find(|candidate| candidate.is_file())
}

fn as_bytes<T>(values: &[T]) -> &[u8] {
    // SAFETY: the callers pass arrays of `f32` and `u32`, which have no
    // padding and no invalid bit patterns, so their bytes are readable.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

/// A point inside the map's open space, found by walking a coarse grid of
/// its bounds and asking the BSP which leaf each point lands in. A leaf with
/// a cluster is one the visibility set covers, which is exactly the space the
/// game lets a player occupy.
fn open_space(world: &source_bsp::World, low: [f32; 3], high: [f32; 3]) -> Option<[f32; 3]> {
    const STEPS: usize = 12;
    let at = |axis: usize, step: usize| {
        let fraction = (step as f32 + 0.5) / STEPS as f32;
        low[axis] + (high[axis] - low[axis]) * fraction
    };
    for z in 0..STEPS {
        for y in 0..STEPS {
            for x in 0..STEPS {
                let point = [at(0, x), at(1, y), at(2, z)];
                let Ok(leaf) = world.point_leaf(point) else {
                    continue;
                };
                if world.leaves()[leaf].cluster >= 0 {
                    return Some(point);
                }
            }
        }
    }
    None
}

#[test]
fn draws_the_world_surfaces_of_a_shipped_map() {
    let Some(map) = find_map() else {
        eprintln!("skipped: no installed Half-Life 2 content to read a map from");
        return;
    };

    let bytes = std::fs::read(&map).expect("the map reads");
    let bsp = Bsp::parse(&bytes).expect("a shipped map parses");
    let surfaces = Surfaces::parse(&bsp).expect("a shipped map has world surfaces");

    // Every world surface at once, which is what a frame eventually draws and
    // what makes this exercise the real index range rather than a handful.
    let geometry = surfaces
        .triangulate(0..surfaces.faces().len())
        .expect("a shipped map triangulates");
    assert!(
        geometry.triangle_count() > 1000,
        "the first map of the campaign has substantial world geometry, got {} triangles",
        geometry.triangle_count()
    );
    assert!(
        geometry.positions.len() < geometry.indices.len(),
        "neighbouring surfaces share corners rather than each carrying their own"
    );

    let device = Device::open().expect("Apple silicon has a Metal device");
    let library = device.compile_library(WORLD_SHADER).expect("compiled");
    let pipeline = device
        .create_depth_pipeline(&library, "world_vertex", "world_fragment")
        .expect("built");

    let vertices = device
        .create_buffer(as_bytes(&geometry.positions))
        .expect("positions upload");
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

    // The middle of a map's bounding box is usually solid rock or the void
    // outside it, so the viewpoint is a point the BSP itself says is open
    // space the visibility set covers, which is where a player can stand.
    let world = source_bsp::World::parse(&bsp).expect("a shipped map has a world");
    let middle = open_space(&world, low, high).expect("a shipped map has somewhere to stand");

    let lens = Lens {
        horizontal_fov: 90.0,
        aspect: 1.0,
        near: 1.0,
        // Far enough to reach the other side of the map from its middle.
        far: (high[0] - low[0]).max(high[1] - low[1]).max(1000.0) * 2.0,
    };
    let black = ClearColor {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alpha: 1.0,
    };

    // Turning on the spot has to find world somewhere: a point inside a map
    // is enclosed by it, so at least one of four quarter turns looks at
    // geometry rather than out of the level.
    let mut best_coverage = 0.0f32;
    for yaw in [0.0, 90.0, 180.0, 270.0] {
        let eye = Eye {
            position: middle,
            angles: [0.0, yaw, 0.0],
        };
        let transform = view_projection(eye, lens).expect("a square lens");

        let drawn = device
            .render_offscreen(
                64,
                64,
                black,
                &[TriangleList::new(
                    &pipeline,
                    Vertices::Buffered {
                        buffer: &vertices,
                        count: geometry.positions.len(),
                        stride: std::mem::size_of::<[f32; 3]>(),
                    },
                )
                .with_indices(Indices {
                    buffer: &indices,
                    count: geometry.indices.len(),
                    format: IndexFormat::Uint32,
                    first: 0,
                })
                .with_uniforms(transform.as_bytes())],
            )
            .expect("the map's world geometry renders");

        let covered = (0..64)
            .flat_map(|y| (0..64).map(move |x| (x, y)))
            .filter(|(x, y)| drawn.pixel(*x, *y) == Some([0, 255, 0, 255]))
            .count();
        best_coverage = best_coverage.max(covered as f32 / (64.0 * 64.0));
    }

    eprintln!(
        "{} triangles from {} shared positions; world filled {:.0}% of the view",
        geometry.triangle_count(),
        geometry.positions.len(),
        best_coverage * 100.0
    );
    assert!(
        best_coverage > 0.5,
        "standing inside the map, world geometry fills most of the view, got {:.0}%",
        best_coverage * 100.0
    );
}
