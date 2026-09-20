//! Decodes every scene in a `scenes.image`, named or not. The differential
//! gate can only ask for scenes whose names it found; this reaches the rest.

use source_scene::cache::SceneCache;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: walk_cache <scenes.image>");
    let cache = SceneCache::load(std::fs::read(&path).expect("read image")).expect("load image");
    let (mut compressed, mut failed) = (0, 0);
    for scene in 0..cache.scene_count() {
        compressed += usize::from(cache.is_compressed(scene) == Some(true));
        match cache.scene_bytes(scene) {
            Some(Ok(bytes)) if Some(bytes.len()) == cache.scene_size(scene) => {}
            _ => {
                failed += 1;
                eprintln!("scene {scene} does not decode to its declared size");
            }
        }
    }
    println!(
        "{} scenes, {compressed} compressed, {failed} failed",
        cache.scene_count()
    );
    std::process::exit(i32::from(failed != 0));
}
