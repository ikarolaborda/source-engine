fn main() {
    // Cargo stamps the library with its absolute build path. Unlike the modules
    // the engine dlopens, this one is named in eight subprojects' link lines, so
    // the name recorded here is the one every consumer carries in LC_LOAD_DYLIB.
    // The launcher puts the game's bin directory on DYLD_LIBRARY_PATH, which dyld
    // searches by leaf name before it resolves the recorded path, so the same
    // @rpath form the sibling modules use resolves here too.
    if std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libsteam_api.dylib");
    }
}
