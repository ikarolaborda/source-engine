//! Reads the models a Half-Life 2 installation ships, rather than ones
//! written by the test.
//!
//! A hand-built model exercises the reader against what the reader's author
//! believed the format to be. The shipped ones exercise it against what the
//! compiler that wrote them actually did, which is where the level-of-detail
//! fixup table matters: a synthetic model has no reason to have one, and the
//! shipped ones almost all do.

use source_filesystem::{Position, SearchPaths};

fn content_root() -> Option<std::path::PathBuf> {
    let root = std::env::var_os("SOURCE_HL2_CONTENT_ROOT")?;
    let root = std::path::PathBuf::from(root);
    root.join("hl2").is_dir().then_some(root)
}

fn game_paths(root: &std::path::Path) -> SearchPaths {
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
    paths
}

/// Props the opening map places, which is where the renderer will meet them
/// first, chosen to cover a static prop, a physics prop and a door.
const MODELS: [&str; 4] = [
    "models/props_c17/door01_left.mdl",
    "models/props_junk/wood_crate001a.mdl",
    "models/props_c17/oildrum001.mdl",
    "models/props_trainstation/trainstation_clock001.mdl",
];

#[test]
fn reads_the_vertices_of_shipped_models_at_every_level_of_detail() {
    let Some(root) = content_root() else {
        eprintln!("skipped: no installed Half-Life 2 content to read models from");
        return;
    };
    let paths = game_paths(&root);

    let mut read = 0;
    let mut with_fixups = 0;
    let mut checked_against_hull = 0;
    let mut jointed = 0;
    let mut posed = 0;
    let mut worst_rest = 0.0f32;
    let mut triangulated = 0;
    let mut triangles_read = 0;
    let mut resolved_materials = 0;
    for name in MODELS {
        let Ok(mdl_bytes) = paths.read(name, None) else {
            continue;
        };
        let Ok(vvd_bytes) = paths.read(&name.replace(".mdl", ".vvd"), None) else {
            continue;
        };
        let mdl = source_studio::Mdl::parse(&mdl_bytes).expect("a shipped model parses");
        let vvd = source_studio::Vvd::parse(&vvd_bytes).expect("its vertices parse");
        assert_eq!(
            mdl.checksum, vvd.checksum,
            "{name} ships vertices built from the model beside them"
        );
        read += 1;
        with_fixups += usize::from(vvd.fixup_count != 0);
        jointed += usize::from(!mdl.is_static_prop());

        let bones = mdl
            .bones(&mdl_bytes)
            .expect("a shipped model's bones parse");
        assert!(!bones.is_empty(), "{name} has at least the one bone");
        assert_eq!(bones[0].parent, -1, "{name} roots its first bone");
        for (index, bone) in bones.iter().enumerate() {
            assert!(
                bone.parent < index as i32,
                "{name} bone {index} hangs off a bone that comes after it"
            );
            let length: f32 = bone.rotation.iter().map(|part| part * part).sum();
            assert!(
                (length - 1.0).abs() < 0.01,
                "{name} bone {index} carries a rotation of length {length}"
            );
        }

        // The skeleton composed from the root down must cancel the
        // transform each bone carries out of model space, because that
        // transform was written by the compiler as the inverse of this
        // pose. It is the strongest statement the file makes about
        // itself, and nothing else catches a quaternion read in the wrong
        // component order, a matrix composed the wrong way round, or a
        // bone hung off the wrong parent: each of those still yields a
        // skeleton, and each places a limb somewhere plausible.
        let pose = mdl
            .bind_pose(&mdl_bytes)
            .expect("a shipped model's skeleton composes");
        assert_eq!(pose.len(), bones.len(), "{name} poses each of its bones");
        let matrices = source_studio::skinning(&pose, &bones);
        for (index, matrix) in matrices.iter().enumerate() {
            let strayed = matrix.distance_from_identity();
            assert!(
                strayed < 1.0e-3,
                "{name} bone {index} skinned at rest is {strayed} from leaving \
                 its vertices alone"
            );
            worst_rest = worst_rest.max(strayed);
        }
        posed += 1;

        // The compiler measured the model into a box and wrote it into the
        // header, so it is an oracle for the decode that the decode had no
        // hand in producing. A reader that lost the fixup table, or took the
        // stride or field order wrong, puts vertices outside it.
        let slack = 1.0;
        for lod in 0..vvd.lod_vertex_counts.len() {
            let vertices = vvd
                .vertices(&vvd_bytes, lod)
                .unwrap_or_else(|error| panic!("{name} level {lod} decodes: {error}"));
            assert_eq!(
                vertices.len(),
                vvd.lod_vertex_counts[lod],
                "{name} level {lod} yields the count its header declares"
            );
            // The compiler measured the model into a box and wrote it
            // into the header, so it is an oracle the decode had no hand
            // in producing: a reader that lost the fixup table, or took
            // the stride or field order wrong, puts vertices outside it.
            //
            // It only speaks for a static prop, and not because a jointed
            // model's vertices are stored anywhere else: the bind-pose
            // check above proves they are in model space too. It is that
            // for a jointed model this field is the movement hull rather
            // than a drawing bound, so a person's arms reach past their
            // 13-by-13 walking box and a door's hull is the volume it
            // sweeps rather than the leaf it is.
            if mdl.is_static_prop() {
                checked_against_hull += 1;
                for vertex in &vertices {
                    for axis in 0..3 {
                        assert!(
                            vertex.position[axis] >= mdl.hull_min[axis] - slack
                                && vertex.position[axis] <= mdl.hull_max[axis] + slack,
                            "{name} level {lod} places a vertex at {:?}, outside the box the \
                             compiler measured, {:?} to {:?}",
                            vertex.position,
                            mdl.hull_min,
                            mdl.hull_max
                        );
                    }
                }
            }

            for vertex in &vertices {
                assert!(
                    vertex.bone_count >= 1 && vertex.bone_count <= 3,
                    "{name} weights a vertex to {} bones",
                    vertex.bone_count
                );
                let total: f32 = vertex.weights[..usize::from(vertex.bone_count)]
                    .iter()
                    .sum();
                assert!(
                    (total - 1.0).abs() < 0.01,
                    "{name} weights a vertex to {total} rather than a whole"
                );
            }
        }

        // A mesh names a material by index, and drawing it means finding
        // that material under one of the directories the model lists, so
        // both halves have to read and the join has to land on a file the
        // installation actually ships.
        let (materials, directories) = mdl.materials(&mdl_bytes).expect("materials parse");
        assert!(!materials.is_empty(), "{name} draws with a material");
        assert!(
            !directories.is_empty(),
            "{name} says where to look for its materials"
        );
        for material in &materials {
            let found = directories.iter().any(|directory| {
                paths
                    .read(&format!("materials/{directory}{material}.vmt"), None)
                    .is_ok()
            });
            assert!(
                found,
                "{name} names material {material:?}, which is under none of {directories:?}"
            );
            resolved_materials += 1;
        }

        // The triangles have to name vertices the vertex file holds, and
        // between them cover most of what it holds: a join that lost the
        // per-mesh base would still produce indices in range on a model
        // whose meshes happen to be small, but would leave the vertices
        // past the first mesh unreferenced.
        if let Ok(vtx_bytes) = paths.read(&name.replace(".mdl", ".dx90.vtx"), None) {
            let vtx = source_studio::Vtx::parse(&vtx_bytes).expect("its triangles parse");
            assert_eq!(
                vtx.checksum, mdl.checksum,
                "{name} ships triangles built from the model beside them"
            );
            let meshes = source_studio::triangles(&mdl, &mdl_bytes, &vtx, &vtx_bytes, 0)
                .unwrap_or_else(|error| panic!("{name} triangulates: {error}"));
            let finest = vvd.lod_vertex_counts[0];
            let mut used = vec![false; finest];
            let mut corners = 0;
            for mesh in &meshes {
                assert_eq!(
                    mesh.indices.len() % 3,
                    0,
                    "{name} mesh {} yields whole triangles",
                    mesh.mesh
                );
                for index in &mesh.indices {
                    let index = *index as usize;
                    assert!(index < finest, "{name} names vertex {index} of {finest}");
                    used[index] = true;
                    corners += 1;
                }
            }
            assert!(corners > 0, "{name} draws something");
            let reached = used.iter().filter(|seen| **seen).count();
            assert!(
                reached * 10 > finest * 9,
                "{name} reaches only {reached} of its {finest} vertices, which is a join \
                 that lost where each mesh's vertices start"
            );
            triangulated += 1;
            triangles_read += corners / 3;
        }

        // The coarser levels are built by leaving stretches of the finest
        // one out, so each is a subset and none is longer than the one
        // before it. Reading the stored run directly would give the right
        // counts and the wrong vertices, which this is what catches.
        let finest: std::collections::HashSet<[u32; 3]> = vvd
            .vertices(&vvd_bytes, 0)
            .expect("the finest level decodes")
            .iter()
            .map(|vertex| vertex.position.map(f32::to_bits))
            .collect();
        for lod in 1..vvd.lod_vertex_counts.len() {
            let coarse = vvd.vertices(&vvd_bytes, lod).expect("a level decodes");
            assert!(
                coarse
                    .iter()
                    .all(|vertex| finest.contains(&vertex.position.map(f32::to_bits))),
                "{name} level {lod} holds a vertex the finest level does not"
            );
        }
    }

    assert!(
        read > 0,
        "an installation that has a map has the props the map places"
    );
    assert!(
        with_fixups > 0,
        "shipped models carry the fixup table this reads, none of {read} did"
    );
    assert!(
        triangulated > 0,
        "the models read yield triangles, none of {read} did"
    );
    assert!(
        checked_against_hull > 0 && jointed > 0,
        "the models read cover both static props and jointed ones, got \
         {checked_against_hull} and {jointed}"
    );
    eprintln!(
        "{read} shipped models decoded, {with_fixups} carrying a fixup table, \
         {checked_against_hull} held against the box the compiler measured, {jointed} jointed, \
         {triangulated} triangulated into {triangles_read} triangles, \
         {resolved_materials} materials resolved, {posed} skeletons composed and \
         cancelling to within {worst_rest:e}"
    );
}

/// Every model the base game ships, held to the one statement a model file
/// makes about its own skeleton that can be checked without a second source.
///
/// The compiler writes each bone's transform out of model space as the
/// inverse of where that bone sits in the rest pose, so composing the
/// skeleton from the root down and multiplying the two must leave every
/// vertex alone. Four models is too small a sample for that: the skeletons
/// that catch a composition order or quaternion order mistake are the deep
/// ones with rotated joints, and the opening map's props are mostly single
/// bones sitting at the origin, which cancel whatever order they are
/// composed in.
#[test]
fn every_shipped_model_composes_a_skeleton_that_cancels_its_own_inverse() {
    let Some(root) = content_root() else {
        eprintln!("skipped: no installed Half-Life 2 content to read models from");
        return;
    };
    let paths = game_paths(&root);

    let mut names: Vec<String> = Vec::new();
    for archive in ["hl2_misc", "hl2_pak"] {
        let path = root.join(format!("hl2/{archive}_dir.vpk"));
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).expect("a shipped archive reads");
        let vpk = source_vpk::Archive::parse(&bytes).expect("a shipped archive parses");
        names.extend(
            vpk.entries()
                .iter()
                .map(|entry| entry.path.clone())
                .filter(|path| path.ends_with(".mdl")),
        );
    }
    names.sort();
    names.dedup();
    assert!(
        names.len() > 1000,
        "the base game ships a library of models, found {}",
        names.len()
    );

    let mut composed = 0usize;
    let mut bones_seen = 0usize;
    let mut deepest = 0usize;
    let mut worst = 0.0f32;
    let mut worst_model = String::new();
    let mut empty = 0usize;
    for name in &names {
        let bytes = paths
            .read(name, None)
            .unwrap_or_else(|error| panic!("{name} reads back out of the archive: {error}"));
        // The archives list six names they hold nothing under, which are
        // the crossbow, the bugbait and the hands: placeholders left where
        // the shipping model was cut. Asserting they are empty rather than
        // skipping whatever fails to parse keeps this from quietly
        // tolerating a real gap in the reader.
        if bytes.is_empty() {
            empty += 1;
            continue;
        }
        let mdl = source_studio::Mdl::parse(&bytes)
            .unwrap_or_else(|error| panic!("{name} parses: {error}"));
        let bones = mdl
            .bones(&bytes)
            .unwrap_or_else(|error| panic!("{name} bones parse: {error}"));
        if bones.is_empty() {
            continue;
        }
        let pose = mdl
            .bind_pose(&bytes)
            .unwrap_or_else(|error| panic!("{name} skeleton composes: {error}"));
        for (index, matrix) in source_studio::skinning(&pose, &bones).iter().enumerate() {
            let strayed = matrix.distance_from_identity();
            // A loose bound rather than an exact one, because the
            // compiler stored the inverse at single precision and a deep
            // skeleton accumulates a little at each joint. Two orders of
            // magnitude below the smallest thing a model measures, so it
            // could not hide a real mistake.
            assert!(
                strayed < 1.0e-2,
                "{name} bone {index} of {} skinned at rest is {strayed} from \
                 leaving its vertices alone",
                bones.len()
            );
            if strayed > worst {
                worst = strayed;
                worst_model = format!("{name} bone {index}");
            }
        }
        let depth = bones
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let mut depth = 0;
                let mut at = index;
                while bones[at].parent >= 0 {
                    at = bones[at].parent as usize;
                    depth += 1;
                }
                depth
            })
            .max()
            .unwrap_or(0);
        deepest = deepest.max(depth);
        bones_seen += bones.len();
        composed += 1;
    }

    assert_eq!(
        composed + empty,
        names.len(),
        "every model the archives list is read, composed or empty"
    );
    assert!(
        deepest >= 10,
        "the sample reaches deep skeletons, whose joints are what a wrong \
         composition order shows up in, deepest was {deepest}"
    );

    eprintln!(
        "{composed} of {} shipped models composed a skeleton, {bones_seen} bones, \
         {deepest} deep at most, cancelling to within {worst:e} at {worst_model}; \
         {empty} listed but empty",
        names.len()
    );
}
