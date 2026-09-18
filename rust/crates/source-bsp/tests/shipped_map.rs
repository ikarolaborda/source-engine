//! Reads a shipped Half-Life 2 map.
//!
//! The unit tests build synthetic lumps, which prove the parser rejects what
//! it should but cannot prove it agrees with what the map compiler actually
//! emitted. Shipped content is not redistributable, so this reports that it
//! was skipped when no installation is present.

use source_bsp::{Bsp, Materials, Surfaces};
use std::path::PathBuf;

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

#[test]
fn resolves_the_materials_a_shipped_map_draws_with() {
    let Some(map) = find_map() else {
        eprintln!("skipped: no installed Half-Life 2 content to read a map from");
        return;
    };

    let bytes = std::fs::read(&map).expect("the map reads");
    let bsp = Bsp::parse(&bytes).expect("a shipped map parses");
    let surfaces = Surfaces::parse(&bsp).expect("a shipped map has world surfaces");
    let materials = Materials::parse(&bsp).expect("a shipped map names its materials");

    assert!(
        !materials.texinfos().is_empty() && !materials.texdatas().is_empty(),
        "a shipped map states how its surfaces are textured"
    );

    // Every drawn surface has to resolve all the way through to a name, which
    // is the whole chain of three lumps agreeing.
    let mut named = 0;
    for texdata in 0..materials.texdatas().len() {
        let name = materials
            .name(texdata)
            .unwrap_or_else(|| panic!("texdata {texdata} names a material"));
        assert!(!name.is_empty(), "texdata {texdata} names something");
        named += 1;
    }
    assert!(named > 10, "a campaign map draws with many materials");

    // Real maps name materials by their path under `materials/`, so a parse
    // that produced rubbish rather than names would not look like this.
    let looks_like_a_path = (0..materials.texdatas().len())
        .filter_map(|texdata| materials.name(texdata))
        .filter(|name| name.contains('/') && name.is_ascii())
        .count();
    assert!(
        looks_like_a_path * 2 > named,
        "most of {named} material names are paths, got {looks_like_a_path}"
    );

    let geometry = surfaces
        .triangulate_textured(&materials, 0..surfaces.faces().len())
        .expect("a shipped map triangulates with texture coordinates");

    assert_eq!(
        geometry.positions.len(),
        geometry.texcoords.len(),
        "every position carries a texture coordinate"
    );
    assert!(
        geometry.triangle_count() > 1000,
        "the first map of the campaign has substantial world geometry"
    );
    assert!(
        geometry.batches.len() > 10,
        "its surfaces group into one run per material"
    );

    // The batches have to partition the indices exactly, or a draw would
    // either miss surfaces or read another material's.
    let mut covered = 0;
    let mut previous_end = 0;
    for batch in &geometry.batches {
        assert_eq!(
            batch.first_index, previous_end,
            "each material's run follows the last without a gap"
        );
        assert_eq!(batch.index_count % 3, 0, "each run is whole triangles");
        assert!(
            materials.name(batch.texdata).is_some(),
            "each run names the material it is drawn with"
        );
        previous_end = batch.first_index + batch.index_count;
        covered += batch.index_count;
    }
    assert_eq!(
        covered,
        geometry.indices.len(),
        "the runs cover every index exactly once"
    );

    // Texture coordinates repeat rather than being clamped to the surface, so
    // they run outside zero to one, but a coordinate in the thousands means
    // the projection was misread rather than tiled.
    // Coordinates tile rather than being clamped to a surface, so they run
    // well outside zero to one: a solid fill like `VGUI/BLACK` is a handful
    // of texels stretched over a wall that seals off a whole area, and
    // legitimately reaches the tens of thousands on this map. What a misread
    // projection produces instead is a coordinate that is not a number, or
    // one with a float's full exponent range behind it.
    let largest = geometry
        .texcoords
        .iter()
        .flatten()
        .fold(0.0f32, |largest, value| largest.max(value.abs()));
    assert!(
        geometry
            .texcoords
            .iter()
            .flatten()
            .all(|coordinate| coordinate.is_finite()),
        "every texture coordinate is a number"
    );
    assert!(
        largest < 1.0e6,
        "texture coordinates tile across a surface rather than running away, got {largest}"
    );

    // Most surfaces tile modestly; only the sealing fills are extreme.
    let modest = geometry
        .texcoords
        .iter()
        .flatten()
        .filter(|coordinate| coordinate.abs() <= 1000.0)
        .count();
    assert!(
        modest * 100 > geometry.texcoords.len() * 2 * 95,
        "all but a few percent of coordinates tile modestly, got {modest}"
    );

    eprintln!(
        "{} triangles over {} vertices in {} material batches, {} materials named",
        geometry.triangle_count(),
        geometry.positions.len(),
        geometry.batches.len(),
        named
    );
}

#[test]
fn reads_what_a_shipped_map_places_and_what_it_can_see() {
    let Some(path) = find_map() else {
        eprintln!("no shipped map to read, set SOURCE_HL2_CONTENT_ROOT");
        return;
    };
    let bytes = std::fs::read(&path).expect("the map reads");
    let bsp = source_bsp::Bsp::parse(&bytes).expect("a shipped map parses");
    let surfaces = source_bsp::Surfaces::parse(&bsp).expect("a shipped map has surfaces");
    let world = source_bsp::World::parse(&bsp).expect("a shipped map has a world");
    let entities = source_bsp::Entities::parse(&bsp).expect("a shipped map places entities");

    // The first model is the map itself and the rest are its doors, lifts
    // and trigger volumes, each named by the entity that places it.
    let models = world.models();
    assert!(models.len() > 1, "a campaign map has brush entities");
    assert_eq!(world.world_faces().start, 0, "the world model comes first");
    assert!(
        world.world_faces().end < surfaces.faces().len(),
        "and the brush models follow it"
    );
    let placed = entities
        .iter()
        .filter_map(source_bsp::Entity::brush_model)
        .collect::<std::collections::HashSet<_>>();
    assert!(
        (1..models.len()).all(|model| placed.contains(&model)),
        "every brush model past the world is placed by an entity"
    );
    assert!(
        !placed.contains(&0),
        "and no entity claims to place the world"
    );

    // The visibility set is what the compiler worked out about the map, and
    // it is symmetric by construction: if one place can see another, the
    // other can see it back. A reader that mistook the run-length encoding
    // would break that long before it drew anything wrong.
    let clusters = world.cluster_count();
    assert!(clusters > 100, "a campaign map has many clusters");
    let rows: Vec<Vec<u8>> = (0..clusters)
        .map(|cluster| {
            world
                .visibility(cluster, source_bsp::VisibilityKind::PotentiallyVisible)
                .expect("every cluster has a visibility row")
        })
        .collect();
    let sees = |from: usize, to: usize| rows[from][to / 8] & (1 << (to & 7)) != 0;
    let mut pairs = 0usize;
    for from in 0..clusters {
        assert!(sees(from, from), "a place can see where it is standing");
        for to in 0..clusters {
            if sees(from, to) {
                pairs += 1;
                assert!(
                    sees(to, from),
                    "cluster {from} sees {to} but not the other way"
                );
            }
        }
    }
    assert!(
        pairs < clusters * clusters / 4,
        "the visibility set narrows the map rather than admitting all of it"
    );

    // Standing where the map starts the player, the visibility set reaches
    // a small part of the world.
    let start = entities
        .by_classname("info_player_start")
        .find_map(|entity| entity.origin())
        .expect("a campaign map starts the player somewhere");
    let standing = [start[0], start[1], start[2] + 48.0];
    let visible = source_bsp::VisibleFaces::select(&world, &surfaces, standing)
        .expect("the player's start is somewhere in the map");
    let drawn = world.world_faces().len();
    assert!(
        !visible.is_empty(),
        "the player can see the room they start in"
    );
    assert!(
        visible.len() * 4 < drawn,
        "and not most of the map, {} of {drawn} surfaces",
        visible.len()
    );

    eprintln!(
        "{} entities placing {} brush models; {clusters} clusters in {pairs} visible pairs; from the player's start {} of {drawn} world surfaces are reachable",
        entities.len(),
        models.len() - 1,
        visible.len(),
    );
}

/// A map is dressed with far more static props than brush entities, and
/// they are not in the entity lump: the compiler moves them into a lump of
/// its own, keyed by a dictionary of model names. This reads a shipped map
/// to check that the lump is walked with the right record stride, which is
/// the thing that quietly goes wrong, because a wrong stride still yields
/// plausible-looking props until they are drawn.
#[test]
fn reads_the_static_props_a_shipped_map_is_dressed_with() {
    let Some(path) = find_map() else {
        eprintln!("no shipped map to read, set SOURCE_HL2_CONTENT_ROOT");
        return;
    };
    let bytes = std::fs::read(&path).expect("the map reads");
    let bsp = source_bsp::Bsp::parse(&bytes).expect("a shipped map parses");
    let world = source_bsp::World::parse(&bsp).expect("a shipped map has a world");
    let props = source_bsp::StaticProps::parse(&bsp).expect("a shipped map's props parse");

    assert!(
        !props.is_empty(),
        "a campaign map is dressed with static props"
    );
    assert!(
        props.props().len() > props.names().len(),
        "the dictionary exists because props share models, {} props over {} models",
        props.props().len(),
        props.names().len()
    );
    for name in props.names() {
        assert!(
            name.starts_with("models/") && name.ends_with(".mdl"),
            "the dictionary holds model paths, not {name:?}"
        );
    }

    // A stride read one field short or long walks into the middle of the
    // next record, and what comes out is angles far outside a circle and
    // origins far outside the map. Both are cheap to state and neither can
    // hold by accident over a map's worth of props.
    let (mut mins, mut maxs) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for leaf in world.leaves() {
        for axis in 0..3 {
            mins[axis] = mins[axis].min(leaf.mins[axis] as f32);
            maxs[axis] = maxs[axis].max(leaf.maxs[axis] as f32);
        }
    }
    let mut standing_in_rooms = 0;
    for (index, prop) in props.props().iter().enumerate() {
        for axis in 0..3 {
            assert!(
                prop.origin[axis] >= mins[axis] && prop.origin[axis] <= maxs[axis],
                "prop {index} stands at {:?}, outside the map's own {mins:?} to {maxs:?}",
                prop.origin
            );
        }
        assert!(
            prop.angles.iter().all(|angle| angle.abs() <= 360.0),
            "prop {index} is turned {:?}, which is not an angle",
            prop.angles
        );
        assert!(
            props.name(prop).is_some(),
            "prop {index} names a model the dictionary holds"
        );
        // The compiler wrote down which leaves each prop stands in, and a
        // prop standing nowhere would be a prop no view could cull.
        let leaves = props.leaves(prop);
        assert!(!leaves.is_empty(), "prop {index} stands in a leaf");
        standing_in_rooms += usize::from(leaves.iter().any(|leaf| {
            world
                .leaves()
                .get(usize::from(*leaf))
                .is_some_and(|leaf| leaf.cluster >= 0)
        }));
    }
    assert!(
        standing_in_rooms * 10 > props.props().len() * 9,
        "static props stand in rooms a view can reach, only {standing_in_rooms} of {} do",
        props.props().len()
    );

    eprintln!(
        "static prop lump version {}: {} props over {} models, {standing_in_rooms} in reachable rooms",
        props.version(),
        props.props().len(),
        props.names().len(),
    );
}

/// A map's doors, lifts and trigger volumes are cut out of the world and
/// stored around an origin of their own, so drawing them as stored piles
/// them all onto the map's origin. This reads a shipped map to check that
/// placing them puts them back where the map says they stand.
#[test]
fn places_brush_models_where_their_entities_stand() {
    let Some(path) = find_map() else {
        eprintln!("no shipped map to read, set SOURCE_HL2_CONTENT_ROOT");
        return;
    };
    let bytes = std::fs::read(&path).expect("the map reads");
    let bsp = source_bsp::Bsp::parse(&bytes).expect("a shipped map parses");
    let surfaces = source_bsp::Surfaces::parse(&bsp).expect("a shipped map has surfaces");
    let world = source_bsp::World::parse(&bsp).expect("a shipped map has a world");
    let materials = source_bsp::Materials::parse(&bsp).expect("a shipped map names materials");
    let entities = source_bsp::Entities::parse(&bsp).expect("a shipped map places entities");

    let placements = entities.brush_placements();
    assert_eq!(
        placements.len(),
        world.models().len() - 1,
        "every brush model past the world is placed exactly once"
    );

    // The map itself says where a door belongs: a door stands in a room, and
    // a room is a leaf the visibility set gave a cluster to. Reading the leaf
    // each model's middle falls in is an oracle the placement had no hand in,
    // and it separates placed geometry from stored geometry sharply, because
    // the origin a stored brush sits on is buried in the map's solid.
    let open = |point: [f32; 3]| {
        world
            .point_leaf(point)
            .ok()
            .and_then(|leaf| world.leaves().get(leaf).copied())
            .is_some_and(|leaf| leaf.cluster >= 0)
    };
    let mut placed_in_rooms = 0;
    let mut stored_in_rooms = 0;
    for (model, placement) in &placements {
        let entry = &world.models()[*model];
        let middle: [f32; 3] =
            std::array::from_fn(|axis| (entry.mins[axis] + entry.maxs[axis]) / 2.0);
        placed_in_rooms += usize::from(open(placement.apply(middle)));
        stored_in_rooms += usize::from(open(middle));
    }
    assert!(
        placed_in_rooms * 4 > placements.len() * 3,
        "placed brush models stand in the map's rooms, only {placed_in_rooms} of {} do",
        placements.len()
    );
    assert!(
        stored_in_rooms * 4 < placements.len(),
        "and drawn as stored they do not, yet {stored_in_rooms} of {} would",
        placements.len()
    );

    let mut groups: Vec<(source_bsp::Placement, Vec<usize>)> = vec![(
        source_bsp::Placement::IDENTITY,
        world.world_faces().collect(),
    )];
    for (model, placement) in &placements {
        groups.push((*placement, world.models()[*model].faces().collect()));
    }
    let placed = surfaces
        .triangulate_placed(&materials, None, groups)
        .expect("a shipped map's world and brush models triangulate together");

    let span = |runs: &[source_bsp::FaceRun], want_placed: bool| {
        let mut mins = [f32::INFINITY; 3];
        let mut maxs = [f32::NEG_INFINITY; 3];
        for run in runs.iter().filter(|run| (run.group != 0) == want_placed) {
            for axis in 0..3 {
                mins[axis] = mins[axis].min(run.mins[axis]);
                maxs[axis] = maxs[axis].max(run.maxs[axis]);
            }
        }
        (mins, maxs)
    };
    let (world_mins, world_maxs) = span(&placed.runs, false);
    let brush_runs = placed.runs.iter().filter(|run| run.group != 0).count();
    assert!(
        brush_runs > 1000,
        "a campaign map's brush entities carry real geometry, got {brush_runs} runs"
    );
    assert!(
        placed
            .runs
            .iter()
            .filter(|run| run.group != 0)
            .all(|run| (0..3).all(
                |axis| run.mins[axis] >= world_mins[axis] && run.maxs[axis] <= world_maxs[axis]
            )),
        "every placed brush surface stands inside the world it was cut from"
    );

    // Drawn as stored, the same surfaces collapse towards the origin, which
    // is the defect this placement exists to fix rather than a difference
    // in how the two are built.
    let stored = surfaces
        .triangulate_textured(&materials, world.world_faces().end..surfaces.faces().len())
        .expect("the same surfaces triangulate without placement");
    let moved = stored
        .runs
        .iter()
        .filter(|run| {
            placed
                .runs
                .iter()
                .find(|placed| placed.face == run.face)
                .is_some_and(|placed| placed.mins != run.mins)
        })
        .count();
    assert!(
        moved * 2 > stored.runs.len(),
        "placing moves most brush surfaces, only {moved} of {} moved",
        stored.runs.len()
    );

    eprintln!(
        "{} brush models placed into {brush_runs} surface runs, {moved} of {} moved by placement",
        placements.len(),
        stored.runs.len(),
    );
}

/// The light the compiler measured inside a shipped map's leaves.
///
/// This is what lights everything that is not a world surface, and unlike
/// a lightmap there is no picture to look at that would show it being read
/// wrong: a cube whose faces are in the wrong order, or whose shared
/// exponent is applied wrongly, still yields numbers that shade a prop to
/// some plausible grey. So it is checked against what light in a room must
/// be true of regardless of the room: that it comes from above, that it
/// varies from place to place, and that the brightest places are the ones
/// open to the sky.
#[test]
fn reads_the_light_inside_a_shipped_map_s_rooms() {
    let Some(path) = find_map() else {
        eprintln!("skipped: no installed Half-Life 2 content to read a map from");
        return;
    };
    let bytes = std::fs::read(&path).expect("a shipped map reads");
    let bsp = source_bsp::Bsp::parse(&bytes).expect("a shipped map parses");
    let world = source_bsp::World::parse(&bsp).expect("a shipped map's tree parses");
    let ambient =
        source_bsp::AmbientLighting::parse(&bsp).expect("a shipped map's ambient light parses");

    assert!(
        !ambient.is_empty(),
        "a compiled campaign map measures the light in its own rooms"
    );

    let mut measured = 0usize;
    let mut from_above = 0usize;
    let mut compared = 0usize;
    let mut peaks: Vec<f32> = Vec::new();
    for (index, leaf) in world.leaves().iter().enumerate() {
        for sample in ambient.samples_in(index) {
            let peak = sample.cube.peak();
            if peak == 0.0 {
                continue;
            }
            measured += 1;
            peaks.push(peak);

            // A room is lit from its lamps and its sky, both of which are
            // above it. Reading the six faces in the wrong order puts that
            // light on a side or underneath.
            let up: f32 = sample.cube.faces[4].iter().sum();
            let down: f32 = sample.cube.faces[5].iter().sum();
            if up != down {
                compared += 1;
                from_above += usize::from(up > down);
            }

            // Every channel of every face has to be a light level rather
            // than the very large or very small number a mishandled shared
            // exponent produces.
            for face in &sample.cube.faces {
                for channel in face {
                    assert!(
                        channel.is_finite() && *channel >= 0.0 && *channel < 1000.0,
                        "leaf {index} is lit to {channel} from one direction"
                    );
                }
            }
        }
        let _ = leaf;
    }

    assert!(
        measured > 1000,
        "the map measures the light through its rooms, got {measured} samples"
    );
    assert!(
        from_above * 4 > compared * 3,
        "light in a room comes from above it, which held in {from_above} of {compared}"
    );

    // A constant would pass everything above. Light varies by orders of
    // magnitude between a lit platform and a closed room, so the spread is
    // what says these are measurements rather than one number repeated.
    peaks.sort_by(f32::total_cmp);
    let median = peaks[peaks.len() / 2];
    let brightest = peaks[peaks.len() - 1];
    let dimmest = peaks[0];
    assert!(
        brightest > median * 4.0,
        "the map's brightest room is brighter than its middling one, {brightest} against {median}"
    );
    assert!(
        dimmest * 4.0 < median,
        "the map's dimmest room is dimmer than its middling one, {dimmest} against {median}"
    );

    eprintln!(
        "{measured} lit samples over {} leaves, lit from above in {from_above} of {compared}, \
         peaks from {dimmest:e} through {median:e} to {brightest:e}{}",
        world.leaves().len(),
        if ambient.is_hdr() {
            ", from the high-range lumps"
        } else {
            ""
        }
    );
}
