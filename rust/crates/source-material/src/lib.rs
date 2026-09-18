//! Source material (`.vmt`) parsing and texture path resolution.
//!
//! A material names a shader and a set of parameters, and the parameters that
//! matter to drawing name textures. Both halves are stated loosely in shipped
//! content: keys and values differ in case between the material and the file
//! it names, separators are backslashes in some materials and forward slashes
//! in others, and a material may be a `patch` that only says how it differs
//! from another one. Everything that reconciles that lives here, so the
//! renderer is handed a path that resolves and a parameter lookup that finds
//! what the material actually set.

use source_keyvalues::{Item, Value};
use std::fmt;

/// Where the engine keeps materials and the textures they name.
pub const MATERIAL_ROOT: &str = "materials";
pub const MATERIAL_SUFFIX: &str = ".vmt";
pub const TEXTURE_SUFFIX: &str = ".vtf";

/// The shader a `patch` material has instead of a real one.
const PATCH_SHADER: &str = "patch";

/// A parsed material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Material {
    shader: String,
    /// Parameter names are stored lowercased because the format treats them
    /// case-insensitively, while values are kept as written since some of
    /// them are paths that are normalized only when used as one.
    parameters: Vec<(String, String)>,
}

impl Material {
    /// Parses one material, which must not be a `patch`.
    ///
    /// A patch is only meaningful once the material it includes has been
    /// read, so it is resolved through [`resolve`] instead.
    pub fn parse(text: &str) -> Result<Self> {
        match parse_either(text)? {
            Parsed::Material(material) => Ok(material),
            Parsed::Patch(patch) => Err(Error::UnresolvedPatch(patch.include)),
        }
    }

    /// The shader name as the material states it, such as
    /// `LightmappedGeneric` for a world surface.
    pub fn shader(&self) -> &str {
        &self.shader
    }

    pub fn parameters(&self) -> impl Iterator<Item = (&str, &str)> {
        self.parameters
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// One parameter's value, found without regard to the case the material
    /// wrote its name in.
    pub fn parameter(&self, name: &str) -> Option<&str> {
        let wanted = name.to_ascii_lowercase();
        self.parameters
            .iter()
            .find(|(key, _)| *key == wanted)
            .map(|(_, value)| value.as_str())
    }

    /// The path of the texture a parameter names, relative to the game
    /// directory, or `None` where the material does not set it.
    ///
    /// Materials name textures without the `materials/` prefix or the `.vtf`
    /// suffix, and write the separator either way round.
    ///
    /// A parameter set to nothing counts as unset, because a few shipped
    /// materials write `"$basetexture" ""` and completing that would name
    /// `materials/.vtf` rather than report that no texture was given.
    pub fn texture_path(&self, parameter: &str) -> Option<String> {
        self.parameter(parameter)
            .filter(|value| !value.trim().is_empty())
            .map(texture_path)
    }

    /// The texture a world surface is drawn with.
    pub fn base_texture(&self) -> Option<String> {
        self.texture_path("$basetexture")
    }

    /// Whether this material is one the world draw skips because the surface
    /// it covers is not a surface at all.
    ///
    /// `nodraw` materials exist so a brush can seal a level without being
    /// visible, and the sky is drawn by the sky box rather than as geometry.
    pub fn is_drawn(&self) -> bool {
        let shader = self.shader.to_ascii_lowercase();
        if shader == "sky" || shader.contains("nodraw") {
            return false;
        }
        // `%compilenodraw` and friends are stated as parameters rather than
        // as a shader, with a value that is a flag.
        for (name, value) in &self.parameters {
            if name.contains("nodraw") && value != "0" {
                return false;
            }
        }
        true
    }
}

/// The path of a texture a material names, relative to the game directory.
pub fn texture_path(value: &str) -> String {
    content_path(value, TEXTURE_SUFFIX)
}

/// The path of a material a map or another material names, relative to the
/// game directory.
pub fn material_path(value: &str) -> String {
    content_path(value, MATERIAL_SUFFIX)
}

/// Completes a content path, adding only the parts the value left out.
///
/// Shipped content states these both ways: a surface names its material
/// without the directory or the suffix, while a patch names the material it
/// includes as the full path. Adding what is already there would produce
/// `materials/materials/...`, which resolves to nothing.
fn content_path(value: &str, suffix: &str) -> String {
    let mut path = normalize(value);
    let prefix = format!("{MATERIAL_ROOT}/");
    if !path.starts_with(&prefix) {
        path.insert_str(0, &prefix);
    }
    if !path.ends_with(suffix) {
        path.push_str(suffix);
    }
    path
}

/// Lowercases and forward-slashes a content path.
///
/// Shipped materials name a texture in whatever case the author typed and
/// with either separator, while the archives store one lowercase
/// forward-slashed form, so a path only resolves once it is reduced to that.
fn normalize(value: &str) -> String {
    value
        .trim()
        .trim_start_matches(['/', '\\'])
        .chars()
        .map(|character| match character {
            '\\' => '/',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// A material stating only how it differs from another one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    /// The material this one is derived from, as a resolvable path.
    pub include: String,
    /// Parameters that overwrite the included material's own.
    pub replace: Vec<(String, String)>,
    /// Parameters added to it.
    pub insert: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Parsed {
    Material(Material),
    Patch(Patch),
}

/// How many materials a chain of patches may include before it is treated as
/// a cycle. Shipped content nests one or two deep.
const MAX_PATCH_DEPTH: usize = 8;

/// Parses a material, following any `patch` it is, by asking `load` for the
/// text of each material included along the way.
///
/// The loader is a closure rather than a filesystem so that resolution stays
/// the caller's concern: the same material text resolves out of a VPK, a
/// loose file, or a test fixture.
pub fn resolve<L>(text: &str, mut load: L) -> Result<Material>
where
    L: FnMut(&str) -> Option<String>,
{
    let mut patches: Vec<Patch> = Vec::new();
    let mut current = parse_either(text)?;

    loop {
        match current {
            Parsed::Material(mut material) => {
                // Applied outermost last, so the material a patch includes is
                // overwritten by the patch rather than the other way round.
                for patch in patches.iter().rev() {
                    material.apply(patch);
                }
                return Ok(material);
            }
            Parsed::Patch(patch) => {
                if patches.len() >= MAX_PATCH_DEPTH {
                    return Err(Error::PatchTooDeep {
                        depth: patches.len(),
                        limit: MAX_PATCH_DEPTH,
                    });
                }
                let included = load(&patch.include)
                    .ok_or_else(|| Error::MissingInclude(patch.include.clone()))?;
                patches.push(patch);
                current = parse_either(&included)?;
            }
        }
    }
}

impl Material {
    fn apply(&mut self, patch: &Patch) {
        for (name, value) in patch.replace.iter().chain(patch.insert.iter()) {
            match self
                .parameters
                .iter_mut()
                .find(|(existing, _)| existing == name)
            {
                Some(slot) => slot.1 = value.clone(),
                None => self.parameters.push((name.clone(), value.clone())),
            }
        }
    }
}

/// Reads material text the way the engine's material system reads it.
///
/// Materials are legacy KeyValues: no escape sequences, so a backslash in a
/// texture path stays a backslash rather than escaping what follows it. An
/// unclosed block is kept rather than refused, because at least one shipped
/// Half-Life 2 material is a closing brace short and the game loads it.
fn parse_material_keyvalues(text: &str) -> Result<source_keyvalues::Document> {
    let options = source_keyvalues::ParseOptions {
        escape_sequences: false,
        keep_unterminated_blocks: true,
        ..Default::default()
    };
    source_keyvalues::parse_with_options(text, options)
        .map_err(|error| Error::Syntax(error.to_string()))
}

fn parse_either(text: &str) -> Result<Parsed> {
    let document = parse_material_keyvalues(text)?;

    let root = document.roots().next().ok_or(Error::Empty)?;
    // A shader given a value rather than a block is refused by the KeyValues
    // reader above, so a root that parsed always has children.
    let children = root.children().unwrap_or_default();

    if root.name.eq_ignore_ascii_case(PATCH_SHADER) {
        return parse_patch(text, children).map(Parsed::Patch);
    }

    let mut parameters = Vec::new();
    for child in children {
        // Nested blocks are per-proxy or per-stage settings that the world
        // draw does not read; only the flat parameters are kept.
        if let Value::String(value) = &child.value {
            parameters.push((child.name.to_ascii_lowercase(), value.clone()));
        }
    }

    Ok(Parsed::Material(Material {
        shader: root.name.clone(),
        parameters,
    }))
}

fn parse_patch(text: &str, children: &[source_keyvalues::Node]) -> Result<Patch> {
    let mut replace = Vec::new();
    let mut insert = Vec::new();
    let mut include = None;

    for child in children {
        match &child.value {
            Value::String(value) if child.name.eq_ignore_ascii_case("include") => {
                include = Some(material_path(value));
            }
            Value::Object(entries) if child.name.eq_ignore_ascii_case("replace") => {
                collect(entries, &mut replace);
            }
            Value::Object(entries) if child.name.eq_ignore_ascii_case("insert") => {
                collect(entries, &mut insert);
            }
            _ => {}
        }
    }

    // A patch may state its include as a top-level `include` directive
    // rather than as a key inside the block, which the KeyValues reader
    // reports separately.
    if include.is_none() {
        let document = parse_material_keyvalues(text)?;
        include = document.items.iter().find_map(|item| match item {
            Item::Include(path) | Item::Base(path) => Some(material_path(path)),
            Item::Node(_) => None,
        });
    }

    Ok(Patch {
        include: include.ok_or(Error::PatchWithoutInclude)?,
        replace,
        insert,
    })
}

fn collect(entries: &[source_keyvalues::Node], into: &mut Vec<(String, String)>) {
    for entry in entries {
        if let Value::String(value) = &entry.value {
            into.push((entry.name.to_ascii_lowercase(), value.clone()));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Syntax(String),
    /// The text held no material at all.
    Empty,
    /// A `patch` reached [`Material::parse`], which cannot follow it.
    UnresolvedPatch(String),
    /// A `patch` that does not say what it patches.
    PatchWithoutInclude,
    /// A material a patch includes could not be loaded.
    MissingInclude(String),
    /// Patches including patches without end.
    PatchTooDeep {
        depth: usize,
        limit: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(message) => write!(f, "material syntax: {message}"),
            Self::Empty => write!(f, "material holds no shader"),
            Self::UnresolvedPatch(include) => {
                write!(f, "material is a patch of {include} and must be resolved")
            }
            Self::PatchWithoutInclude => write!(f, "material patch names nothing to patch"),
            Self::MissingInclude(path) => write!(f, "material patch includes missing {path}"),
            Self::PatchTooDeep { depth, limit } => {
                write!(f, "material patch chain of {depth} exceeds {limit}")
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    /// A world material as shipped content actually writes one, taken from
    /// `materials/brick/brickwall001a.vmt`.
    const BRICK: &str = r#"
"LightmappedGeneric"
{
	"$basetexture" "Brick/brickwall001a"
	"$surfaceprop" "brick"

	"$detail" "detail\noise_detail_01"
	"$detailscale" "7.740"
	"$detailblendfactor" .8

	"$detailblendmode" 0
	"%keywords" "c17downtown"
}
"#;

    #[test]
    fn reads_the_shader_and_parameters_a_world_material_states() {
        let material = Material::parse(BRICK).unwrap();

        assert_eq!(material.shader(), "LightmappedGeneric");
        assert_eq!(material.parameter("$surfaceprop"), Some("brick"));
        // The format is case-insensitive about parameter names, and shipped
        // content is inconsistent about them.
        assert_eq!(material.parameter("$SurfaceProp"), Some("brick"));
        assert_eq!(
            material.parameter("$BaseTexture"),
            Some("Brick/brickwall001a")
        );
        assert_eq!(material.parameter("$nope"), None);
        assert!(material.is_drawn());
    }

    #[test]
    fn resolves_a_named_texture_to_a_path_that_exists_in_the_archives() {
        let material = Material::parse(BRICK).unwrap();

        // The archives hold one lowercase forward-slashed path, while the
        // material names the texture in the author's own case, without the
        // directory it lives in and without a suffix.
        assert_eq!(
            material.base_texture().as_deref(),
            Some("materials/brick/brickwall001a.vtf")
        );
        // And a parameter written with the other separator resolves too.
        assert_eq!(
            material.texture_path("$detail").as_deref(),
            Some("materials/detail/noise_detail_01.vtf")
        );
        assert_eq!(material.texture_path("$nope"), None);

        // A few shipped debug materials set the parameter to nothing, which
        // names no texture rather than naming `materials/.vtf`.
        let blank = Material::parse(r#""UnlitGeneric" { "$basetexture" "" }"#).unwrap();
        assert_eq!(blank.parameter("$basetexture"), Some(""));
        assert_eq!(blank.base_texture(), None);
    }

    #[test]
    fn normalizes_paths_the_way_the_archives_store_them() {
        assert_eq!(texture_path("Brick\\Wall"), "materials/brick/wall.vtf");
        assert_eq!(texture_path("  spaced  "), "materials/spaced.vtf");
        assert_eq!(texture_path("/leading"), "materials/leading.vtf");
        // A value that already carries its suffix is not given a second one.
        assert_eq!(texture_path("thing.vtf"), "materials/thing.vtf");
        assert_eq!(material_path("Metal/Wall"), "materials/metal/wall.vmt");
        assert_eq!(material_path("metal/wall.vmt"), "materials/metal/wall.vmt");
    }

    #[test]
    fn keeps_the_parameters_a_material_set_and_no_others() {
        let material = Material::parse(BRICK).unwrap();
        let names: Vec<&str> = material.parameters().map(|(name, _)| name).collect();

        assert!(names.contains(&"$basetexture"));
        assert!(names.contains(&"%keywords"));
        assert_eq!(names.len(), 7, "the material states seven parameters");
    }

    #[test]
    fn follows_a_patch_to_the_material_it_derives_from() {
        let patch = r#"
"patch"
{
	"include" "materials/brick/brickwall001a.vmt"
	"replace"
	{
		"$basetexture" "Brick/other"
	}
	"insert"
	{
		"$alpha" "0.5"
	}
}
"#;
        let material = resolve(patch, |path| {
            assert_eq!(path, "materials/brick/brickwall001a.vmt");
            Some(BRICK.to_owned())
        })
        .unwrap();

        // The shader and every unpatched parameter come from the included
        // material, the replaced one from the patch, and the inserted one is
        // new.
        assert_eq!(material.shader(), "LightmappedGeneric");
        assert_eq!(material.parameter("$surfaceprop"), Some("brick"));
        assert_eq!(material.parameter("$basetexture"), Some("Brick/other"));
        assert_eq!(material.parameter("$alpha"), Some("0.5"));
        assert_eq!(
            material.base_texture().as_deref(),
            Some("materials/brick/other.vtf")
        );
    }

    #[test]
    fn lets_the_outermost_patch_win_a_chain() {
        let outer = r#"
"patch"
{
	"include" "middle"
	"replace" { "$basetexture" "outer" }
}
"#;
        let middle = r#"
"patch"
{
	"include" "materials/brick/brickwall001a.vmt"
	"replace" { "$basetexture" "middle" "$surfaceprop" "metal" }
}
"#;
        let material = resolve(outer, |path| match path {
            "materials/middle.vmt" => Some(middle.to_owned()),
            "materials/brick/brickwall001a.vmt" => Some(BRICK.to_owned()),
            other => panic!("unexpected include {other}"),
        })
        .unwrap();

        assert_eq!(material.parameter("$basetexture"), Some("outer"));
        // What only the middle patch changed still takes effect.
        assert_eq!(material.parameter("$surfaceprop"), Some("metal"));
    }

    #[test]
    fn refuses_a_patch_it_cannot_resolve() {
        let patch = r#""patch" { "include" "missing" }"#;

        assert_eq!(
            Material::parse(patch),
            Err(Error::UnresolvedPatch("materials/missing.vmt".to_owned()))
        );
        assert_eq!(
            resolve(patch, |_| None),
            Err(Error::MissingInclude("materials/missing.vmt".to_owned()))
        );
        assert_eq!(
            resolve(r#""patch" { "replace" { "$a" "1" } }"#, |_| None),
            Err(Error::PatchWithoutInclude)
        );

        // A patch that includes itself would otherwise never finish.
        let cycle = r#""patch" { "include" "loop" }"#;
        assert_eq!(
            resolve(cycle, |_| Some(cycle.to_owned())),
            Err(Error::PatchTooDeep {
                depth: MAX_PATCH_DEPTH,
                limit: MAX_PATCH_DEPTH,
            })
        );
    }

    #[test]
    fn knows_which_materials_a_world_draw_shows() {
        assert!(!Material::parse(r#""Sky" { "$basetexture" "sky" }"#)
            .unwrap()
            .is_drawn());
        assert!(
            !Material::parse(r#""UnlitGeneric" { "%compileNoDraw" "1" }"#)
                .unwrap()
                .is_drawn()
        );
        // A flag that is set to off is not a reason to skip the surface.
        assert!(
            Material::parse(r#""UnlitGeneric" { "%compileNoDraw" "0" }"#)
                .unwrap()
                .is_drawn()
        );
        assert!(Material::parse(BRICK).unwrap().is_drawn());
    }

    #[test]
    fn refuses_text_that_is_not_a_material() {
        assert_eq!(Material::parse(""), Err(Error::Empty));
        // A shader given a value rather than a parameter block is not a
        // material at all, and the KeyValues reader says so first.
        assert!(matches!(
            Material::parse(r#""LightmappedGeneric" "value""#),
            Err(Error::Syntax(_))
        ));
    }

    #[test]
    fn loads_a_material_that_is_a_closing_brace_short() {
        // `materials/models/props_lab/tank_glass001_dx60.vmt` as shipped:
        // its proxy block closes but the material block never does. The
        // engine reports that and keeps the keys it read, so the material
        // works in the game and has to work here.
        let shipped = r#"
"UnlitTwoTexture"
{
	"$basetexture" "dev/water"
	"$texture2" "models/props_lab/glass_tint001"
	"Proxies"
	{
		"AnimatedTexture"
		{
			"animatedtexturevar" "$basetexture"
		}
}
"#;

        let material = Material::parse(shipped).unwrap();

        assert_eq!(material.shader(), "UnlitTwoTexture");
        assert_eq!(
            material.base_texture().as_deref(),
            Some("materials/dev/water.vtf")
        );
    }

    #[test]
    fn describes_every_failure() {
        for error in [
            Error::Syntax("bad".to_owned()),
            Error::Empty,
            Error::UnresolvedPatch("a".to_owned()),
            Error::PatchWithoutInclude,
            Error::MissingInclude("a".to_owned()),
            Error::PatchTooDeep { depth: 9, limit: 8 },
        ] {
            assert!(!error.to_string().is_empty());
        }
    }
}
