fn main() {
    // Cargo stamps the library with its absolute build path; the engine loads
    // it by name from its own bin directory.
    if std::env::var("CARGO_CFG_TARGET_VENDOR").as_deref() == Ok("apple") {
        println!(
            "cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libsoundemittersystem.dylib"
        );
    }
}
