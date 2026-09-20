//! The resident scene cache: the lookups `scenefilecache/SceneFileCache.cpp`
//! answers from a loaded `scenes/scenes.image`.
//!
//! [`SceneImage`](crate::SceneImage) validates a whole image and refuses one
//! that departs from what the shipped compiler writes. That is the wrong gate
//! for the running game, where the native cache checks the tag and the version
//! and then trusts every offset: an image another tool laid out differently
//! plays natively and must keep playing here. This view accepts those images
//! and reads them the same way, but checks each offset against the buffer, so
//! a lookup the native code would answer from memory it does not own is
//! answered as not found. Where that and the decoder make it differ from the
//! native module is listed in `rust/README.md`.

use crate::{SCENE_IMAGE_MAGIC, SCENE_IMAGE_VERSION};
use source_compress::lzma;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::ffi::CStr;
use std::fmt;

/// `MAX_PATH`, the size of the buffer the native cache normalizes a name in.
const NAME_BUFFER: usize = 260;
const HEADER_BYTES: usize = 20;
const ENTRY_BYTES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Shorter than a header, or not tagged `VSIF` version 2.
    BadHeader,
    /// The scene directory does not fit in the file.
    DirectoryOutOfBounds,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadHeader => f.write_str("not a version 2 scene image"),
            Self::DirectoryOutOfBounds => f.write_str("scene directory lies outside the image"),
        }
    }
}

impl std::error::Error for LoadError {}

/// `SceneCachedData_t` without the scene index the caller already holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedData {
    pub milliseconds: u32,
    pub sound_count: i32,
}

#[derive(Debug)]
pub struct SceneCache {
    image: Vec<u8>,
    scene_count: usize,
    string_count: usize,
    entry_offset: usize,
}

impl SceneCache {
    pub fn load(image: Vec<u8>) -> Result<Self, LoadError> {
        if image.len() < HEADER_BYTES
            || read_u32(&image, 0) != Some(SCENE_IMAGE_MAGIC)
            || read_i32(&image, 4) != Some(SCENE_IMAGE_VERSION)
        {
            return Err(LoadError::BadHeader);
        }
        // A negative count makes every native search and range check fail,
        // which is what no scenes and no strings do.
        let scene_count = read_i32(&image, 8).map_or(0, |count| count.max(0) as usize);
        let string_count = read_i32(&image, 12).map_or(0, |count| count.max(0) as usize);
        // With no scenes the directory is never read, so where the header says
        // it is does not matter, as it does not natively.
        let entry_offset = read_i32(&image, 16)
            .and_then(|offset| usize::try_from(offset).ok())
            .unwrap_or(usize::MAX);
        let directory_end = scene_count
            .checked_mul(ENTRY_BYTES)
            .and_then(|size| entry_offset.checked_add(size));
        if scene_count > 0 && directory_end.is_none_or(|end| end > image.len()) {
            return Err(LoadError::DirectoryOutOfBounds);
        }
        Ok(Self {
            image,
            scene_count,
            string_count,
            entry_offset,
        })
    }

    pub fn scene_count(&self) -> usize {
        self.scene_count
    }

    /// `FindSceneInImage`: the same binary search over the directory as it is
    /// stored, so an image that is not sorted misses exactly where native does.
    pub fn find(&self, name: &[u8]) -> Option<usize> {
        let wanted = name_crc(name);
        let mut lower = 1usize;
        let mut upper = self.scene_count;
        while lower <= upper {
            let middle = (lower + upper) / 2;
            let probe = read_u32(&self.image, self.entry_offset + (middle - 1) * ENTRY_BYTES)?;
            match wanted.cmp(&probe) {
                Ordering::Less => upper = middle - 1,
                Ordering::Greater => lower = middle + 1,
                Ordering::Equal => return Some(middle - 1),
            }
        }
        None
    }

    /// The summary behind `GetSceneCachedData`.
    pub fn cached_data(&self, scene: usize) -> Option<CachedData> {
        let summary = self.summary_offset(scene)?;
        Some(CachedData {
            milliseconds: read_u32(&self.image, summary)?,
            sound_count: read_i32(&self.image, summary.checked_add(4)?)?,
        })
    }

    /// `GetSceneCachedSound`, including its narrowing of the stored `int` to
    /// the `short` the interface returns.
    pub fn cached_sound(&self, scene: i32, sound: i32) -> Option<i16> {
        let scene = usize::try_from(scene).ok()?;
        let summary = self.summary_offset(scene)?;
        let count = read_i32(&self.image, summary.checked_add(4)?)?;
        if sound < 0 || sound >= count {
            return None;
        }
        let offset = summary.checked_add(8 + (sound as usize).checked_mul(4)?)?;
        Some(read_i32(&self.image, offset)? as i16)
    }

    /// `GetSceneString`. The string stays where it is in the image, so the
    /// pointer the engine is handed lives exactly as long as the image does.
    pub fn string(&self, id: i16) -> Option<&CStr> {
        let id = usize::try_from(id).ok()?;
        if id >= self.string_count {
            return None;
        }
        let offset = read_u32(&self.image, HEADER_BYTES + id * 4)? as usize;
        CStr::from_bytes_until_nul(self.image.get(offset..)?).ok()
    }

    /// The length `GetSceneBufferSize` reports: what the scene decompresses
    /// to, or what it occupies when it is stored as it is.
    pub fn scene_size(&self, scene: usize) -> Option<usize> {
        let (blob, stored_length) = self.blob(scene)?;
        Some(if lzma::is_compressed(blob) {
            lzma::actual_size(blob)
        } else {
            stored_length
        })
    }

    /// `GetSceneDataFromImage` with a destination: fills as much of `output`
    /// as the scene has, and returns the scene's whole length either way. A
    /// compressed scene that does not decode leaves `output` alone and still
    /// reports its declared length, as the native cache ignores that failure.
    pub fn copy_scene(&self, scene: usize, output: &mut [u8]) -> Option<usize> {
        let (blob, stored_length) = self.blob(scene)?;
        if lzma::is_compressed(blob) {
            if let Ok(data) = lzma::decompress(blob) {
                let count = data.len().min(output.len());
                output[..count].copy_from_slice(&data[..count]);
            }
            return Some(lzma::actual_size(blob));
        }
        let count = blob.len().min(stored_length).min(output.len());
        output[..count].copy_from_slice(&blob[..count]);
        Some(stored_length)
    }

    /// A scene's bytes as the engine will parse them, decompressed if need
    /// be. `None` is a scene that is not there; `Err` is one whose stream
    /// does not decode.
    pub fn scene_bytes(&self, scene: usize) -> Option<Result<Cow<'_, [u8]>, lzma::Error>> {
        let (blob, stored_length) = self.blob(scene)?;
        Some(if lzma::is_compressed(blob) {
            lzma::decompress(blob).map(Cow::Owned)
        } else {
            Ok(Cow::Borrowed(&blob[..blob.len().min(stored_length)]))
        })
    }

    /// Whether the scene is stored compressed.
    pub fn is_compressed(&self, scene: usize) -> Option<bool> {
        self.blob(scene).map(|(blob, _)| lzma::is_compressed(blob))
    }

    fn entry(&self, scene: usize) -> Option<usize> {
        (scene < self.scene_count).then(|| self.entry_offset + scene * ENTRY_BYTES)
    }

    fn summary_offset(&self, scene: usize) -> Option<usize> {
        let entry = self.entry(scene)?;
        usize::try_from(read_i32(&self.image, entry + 12)?).ok()
    }

    /// The bytes from the scene's data offset to the end of the image, and the
    /// length its directory entry declares. Compressed scenes are measured by
    /// their own header, as the native cache measures them.
    fn blob(&self, scene: usize) -> Option<(&[u8], usize)> {
        let entry = self.entry(scene)?;
        let offset = usize::try_from(read_i32(&self.image, entry + 4)?).ok()?;
        let length = read_i32(&self.image, entry + 8)? as usize;
        Some((self.image.get(offset..)?, length))
    }
}

/// The checksum a scene is filed under: `CRC32_ProcessSingleBuffer` of its
/// name after [`normalize_name`].
pub fn name_crc(name: &[u8]) -> u32 {
    source_binary::crc32(&normalize_name(name))
}

/// What `FindSceneInImage` does to a name before hashing it: truncate to the
/// native buffer, lowercase, turn every separator into a backslash, and force
/// the extension to `.vcd` with `V_SetExtension`'s exact rules: strip what
/// follows the last dot of the final component, then append.
pub fn normalize_name(name: &[u8]) -> Vec<u8> {
    let end = name
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(name.len())
        .min(NAME_BUFFER - 1);
    let mut clean: Vec<u8> = name[..end]
        .iter()
        .map(|byte| match byte.to_ascii_lowercase() {
            b'/' => b'\\',
            lowered => lowered,
        })
        .collect();

    // V_StripExtension: cut at the last dot unless a separator, or the start
    // of the name, comes first.
    let mut last = clean.len().saturating_sub(1);
    while last > 0 && clean[last] != b'.' && clean[last] != b'\\' {
        last -= 1;
    }
    if last > 0 && clean[last] != b'\\' {
        clean.truncate(last);
    }

    // V_SetExtension then appends unconditionally, keeping whatever part of
    // the extension still fits the buffer.
    let room = (NAME_BUFFER - 1).saturating_sub(clean.len());
    clean.extend_from_slice(&b".vcd"[..room.min(4)]);
    clean
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let field = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

fn read_i32(bytes: &[u8], offset: usize) -> Option<i32> {
    read_u32(bytes, offset).map(|value| value as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two scenes and two strings, laid out with gaps the strict parser
    /// refuses and the native cache does not notice.
    /// Name, stored bytes, milliseconds, sound string indices.
    type Scene<'a> = (&'a [u8], &'a [u8], u32, &'a [i32]);

    fn image(scenes: &[Scene]) -> Vec<u8> {
        let strings: [&[u8]; 2] = [b"first.wav\0", b"second.wav\0"];
        let mut sorted: Vec<_> = scenes.to_vec();
        sorted.sort_by_key(|scene| name_crc(scene.0));

        let mut bytes = vec![0u8; HEADER_BYTES + strings.len() * 4];
        for (index, string) in strings.iter().enumerate() {
            let offset = bytes.len() as u32;
            bytes[HEADER_BYTES + index * 4..][..4].copy_from_slice(&offset.to_le_bytes());
            bytes.extend_from_slice(string);
        }
        bytes.extend_from_slice(&[0xaa; 3]);
        let entry_offset = bytes.len();
        bytes.resize(entry_offset + sorted.len() * ENTRY_BYTES, 0);
        for (index, (name, data, milliseconds, sounds)) in sorted.iter().enumerate() {
            bytes.extend_from_slice(&[0xbb; 5]);
            let summary = bytes.len();
            bytes.extend_from_slice(&milliseconds.to_le_bytes());
            bytes.extend_from_slice(&(sounds.len() as i32).to_le_bytes());
            for sound in *sounds {
                bytes.extend_from_slice(&sound.to_le_bytes());
            }
            let data_offset = bytes.len();
            bytes.extend_from_slice(data);
            let entry = entry_offset + index * ENTRY_BYTES;
            bytes[entry..][..4].copy_from_slice(&name_crc(name).to_le_bytes());
            bytes[entry + 4..][..4].copy_from_slice(&(data_offset as i32).to_le_bytes());
            bytes[entry + 8..][..4].copy_from_slice(&(data.len() as i32).to_le_bytes());
            bytes[entry + 12..][..4].copy_from_slice(&(summary as i32).to_le_bytes());
        }
        bytes[0..4].copy_from_slice(b"VSIF");
        bytes[4..8].copy_from_slice(&2i32.to_le_bytes());
        bytes[8..12].copy_from_slice(&(sorted.len() as i32).to_le_bytes());
        bytes[12..16].copy_from_slice(&(strings.len() as i32).to_le_bytes());
        bytes[16..20].copy_from_slice(&(entry_offset as i32).to_le_bytes());
        bytes
    }

    fn two_scenes() -> SceneCache {
        SceneCache::load(image(&[
            (b"scenes/a.vcd", b"bvcd-first", 1500, &[1, 0]),
            (b"scenes/sub/b.vcd", b"bvcd-second-longer", 20, &[]),
        ]))
        .unwrap()
    }

    #[test]
    fn normalizes_names_as_the_native_cache_does() {
        let cases: [(&[u8], &[u8]); 10] = [
            (b"scenes/Foo/Bar.VCD", b"scenes\\foo\\bar.vcd"),
            (b"scenes\\foo\\bar", b"scenes\\foo\\bar.vcd"),
            (b"scenes/foo/bar.txt", b"scenes\\foo\\bar.vcd"),
            (b"scenes/foo.d/bar", b"scenes\\foo.d\\bar.vcd"),
            (b"scenes/foo/bar.", b"scenes\\foo\\bar.vcd"),
            (b"a.b.c", b"a.b.vcd"),
            (b"scenes/x/foo.v2.vcd", b"scenes\\x\\foo.v2.vcd"),
            (b".hidden", b".hidden.vcd"),
            (b"", b".vcd"),
            (b"scenes/foo\0ignored", b"scenes\\foo.vcd"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_name(input),
                expected,
                "{}",
                String::from_utf8_lossy(input)
            );
        }
    }

    #[test]
    fn truncates_long_names_to_the_native_buffer() {
        let long = vec![b'x'; 400];
        let clean = normalize_name(&long);
        assert_eq!(clean.len(), NAME_BUFFER - 1);
        assert!(clean.iter().all(|byte| *byte == b'x'));

        // Three bytes of room take three bytes of the extension.
        let mut nearly = vec![b'y'; NAME_BUFFER - 4];
        assert_eq!(normalize_name(&nearly)[NAME_BUFFER - 4..], *b".vc");
        nearly.extend_from_slice(b".vcd");
        assert_eq!(normalize_name(&nearly)[NAME_BUFFER - 4..], *b".vc");
    }

    #[test]
    fn answers_the_interface_queries() {
        let cache = two_scenes();
        let a = cache.find(b"SCENES\\A").unwrap();
        let b = cache.find(b"scenes/sub/b.vcd").unwrap();
        assert_ne!(a, b);
        assert_eq!(cache.find(b"scenes/missing.vcd"), None);

        assert_eq!(
            cache.cached_data(a),
            Some(CachedData {
                milliseconds: 1500,
                sound_count: 2
            })
        );
        assert_eq!(cache.cached_sound(a as i32, 0), Some(1));
        assert_eq!(cache.cached_sound(a as i32, 1), Some(0));
        assert_eq!(cache.cached_sound(a as i32, 2), None);
        assert_eq!(cache.cached_sound(a as i32, -1), None);
        assert_eq!(cache.cached_sound(b as i32, 0), None);
        assert_eq!(cache.cached_sound(-1, 0), None);
        assert_eq!(cache.cached_sound(2, 0), None);

        assert_eq!(cache.string(0).unwrap().to_bytes(), b"first.wav");
        assert_eq!(cache.string(1).unwrap().to_bytes(), b"second.wav");
        assert_eq!(cache.string(2), None);
        assert_eq!(cache.string(-1), None);

        assert_eq!(cache.scene_size(a), Some(10));
        let mut whole = [0u8; 32];
        assert_eq!(cache.copy_scene(a, &mut whole), Some(10));
        assert_eq!(&whole[..10], b"bvcd-first");
        let mut short = [0u8; 4];
        assert_eq!(cache.copy_scene(b, &mut short), Some(18));
        assert_eq!(&short, b"bvcd");
        assert_eq!(cache.copy_scene(2, &mut short), None);
    }

    #[test]
    fn accepts_the_layout_the_strict_parser_refuses() {
        let bytes = image(&[(b"scenes/a.vcd", b"bvcd-first", 1, &[])]);
        assert!(crate::SceneImage::parse(&bytes).is_err());
        assert!(SceneCache::load(bytes).is_ok());
    }

    #[test]
    fn refuses_what_the_native_cache_refuses_and_bounds_the_rest() {
        assert_eq!(
            SceneCache::load(b"VSIF".to_vec()).unwrap_err(),
            LoadError::BadHeader
        );
        let mut wrong_version = image(&[]);
        wrong_version[4] = 3;
        assert_eq!(
            SceneCache::load(wrong_version).unwrap_err(),
            LoadError::BadHeader
        );
        let mut overlong = image(&[(b"scenes/a.vcd", b"x", 1, &[])]);
        overlong[8..12].copy_from_slice(&1_000_000i32.to_le_bytes());
        assert_eq!(
            SceneCache::load(overlong).unwrap_err(),
            LoadError::DirectoryOutOfBounds
        );

        // Offsets that leave the image are misses, not reads.
        let mut stray = image(&[(b"scenes/a.vcd", b"bvcd", 1, &[0])]);
        let entry = i32::from_le_bytes(stray[16..20].try_into().unwrap()) as usize;
        stray[entry + 4..][..4].copy_from_slice(&i32::MAX.to_le_bytes());
        stray[entry + 12..][..4].copy_from_slice(&(-8i32).to_le_bytes());
        let string_table = HEADER_BYTES;
        stray[string_table..][..4].copy_from_slice(&u32::MAX.to_le_bytes());
        let cache = SceneCache::load(stray).unwrap();
        let scene = cache.find(b"scenes/a").unwrap();
        assert_eq!(cache.scene_size(scene), None);
        assert_eq!(cache.cached_data(scene), None);
        assert_eq!(cache.cached_sound(scene as i32, 0), None);
        assert_eq!(cache.string(0), None);

        let mut negative = image(&[(b"scenes/a.vcd", b"x", 1, &[])]);
        negative[8..12].copy_from_slice(&(-5i32).to_le_bytes());
        let cache = SceneCache::load(negative).unwrap();
        assert_eq!(cache.scene_count(), 0);
        assert_eq!(cache.find(b"scenes/a"), None);
    }

    #[test]
    fn decodes_compressed_scenes_and_survives_corrupt_ones() {
        let payload: Vec<u8> = (0..900u32).map(|value| (value % 11) as u8).collect();
        let mut stream = Vec::new();
        lzma_rs::lzma_compress_with_options(
            &mut &payload[..],
            &mut stream,
            &lzma_rs::compress::Options {
                unpacked_size: lzma_rs::compress::UnpackedSize::SkipWritingToHeader,
            },
        )
        .unwrap();
        let mut blob = b"LZMA".to_vec();
        blob.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        blob.extend_from_slice(&((stream.len() - 5) as u32).to_le_bytes());
        blob.extend_from_slice(&stream);

        let cache = SceneCache::load(image(&[(b"scenes/z.vcd", &blob, 7, &[])])).unwrap();
        let scene = cache.find(b"scenes/z").unwrap();
        assert_eq!(cache.scene_size(scene), Some(payload.len()));
        let mut whole = vec![0u8; payload.len()];
        assert_eq!(cache.copy_scene(scene, &mut whole), Some(payload.len()));
        assert_eq!(whole, payload);
        let mut part = vec![0u8; 100];
        assert_eq!(cache.copy_scene(scene, &mut part), Some(payload.len()));
        assert_eq!(part, payload[..100]);

        let mut corrupt = blob.clone();
        let last = corrupt.len() - 1;
        corrupt.truncate(last - 40);
        corrupt.extend_from_slice(&[0xff; 40]);
        let cache = SceneCache::load(image(&[(b"scenes/z.vcd", &corrupt, 7, &[])])).unwrap();
        let mut untouched = vec![0x55u8; payload.len()];
        let reported = cache.copy_scene(0, &mut untouched);
        assert_eq!(reported, Some(payload.len()));
    }
}
