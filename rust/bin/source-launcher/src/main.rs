//! Rust-owned process entry for the staged macOS engine port.
//!
//! The legacy launcher remains a dynamically loaded callback until its module
//! startup body has migrated. Argument storage and the library handle stay live
//! for the entire callback, and no Rust allocation crosses the C boundary.

#[cfg(unix)]
mod unix {
    use std::env;
    use std::ffi::{c_char, c_int, c_void, CStr, CString, OsString};
    use std::io::{self, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::ptr;

    const RTLD_NOW: c_int = 0x2;
    const RTLD_LOCAL: c_int = 0x4;

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
    }

    type LegacyLauncherMain = unsafe extern "C" fn(c_int, *mut *mut c_char) -> c_int;

    fn library_filename() -> &'static str {
        if cfg!(target_os = "macos") {
            "liblauncher.dylib"
        } else {
            "liblauncher.so"
        }
    }

    fn installed_launcher_path(executable: &Path) -> Result<PathBuf, String> {
        let executable_dir = executable
            .parent()
            .ok_or_else(|| "the executable has no parent directory".to_owned())?;
        Ok(executable_dir.join("bin").join(library_filename()))
    }

    fn cstring(value: &Path) -> Result<CString, String> {
        CString::new(value.as_os_str().as_bytes()).map_err(|_| {
            format!(
                "launcher path contains an interior NUL byte: {}",
                value.display()
            )
        })
    }

    fn c_arguments(arguments: Vec<OsString>) -> Result<(Vec<CString>, Vec<*mut c_char>), String> {
        if arguments.len() > c_int::MAX as usize {
            return Err("argument count exceeds the C launcher limit".to_owned());
        }
        let owned = arguments
            .into_iter()
            .map(|argument| {
                CString::new(argument.as_os_str().as_bytes())
                    .map_err(|_| "an argument contains an interior NUL byte".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut pointers = owned
            .iter()
            .map(|argument| argument.as_ptr().cast_mut())
            .collect::<Vec<_>>();
        pointers.push(ptr::null_mut());
        Ok((owned, pointers))
    }

    fn dynamic_loader_error(action: &str) -> String {
        // SAFETY: `dlerror` returns either null or a process-owned NUL-terminated string.
        let message = unsafe {
            let error = dlerror();
            if error.is_null() {
                "unknown dynamic-loader error".into()
            } else {
                CStr::from_ptr(error).to_string_lossy().into_owned()
            }
        };
        format!("{action}: {message}")
    }

    pub fn run() -> Result<c_int, String> {
        let executable = env::current_exe()
            .map_err(|error| format!("failed to resolve the executable path: {error}"))?;
        let launcher_path = installed_launcher_path(&executable)?;
        let launcher_path_c = cstring(&launcher_path)?;

        // SAFETY: the path is NUL-terminated and the returned handle is checked before use.
        let launcher = unsafe { dlopen(launcher_path_c.as_ptr(), RTLD_NOW | RTLD_LOCAL) };
        if launcher.is_null() {
            return Err(dynamic_loader_error(&format!(
                "failed to load {}",
                launcher_path.display()
            )));
        }

        let symbol_name = c"LauncherMain";
        // Clear a stale error before resolving the symbol, then inspect both the pointer and
        // loader state. POSIX permits a valid symbol value to be null in principle.
        unsafe {
            dlerror();
        }
        // SAFETY: `launcher` is a live handle and `symbol_name` is NUL-terminated.
        let symbol = unsafe { dlsym(launcher, symbol_name.as_ptr()) };
        // SAFETY: see `dynamic_loader_error`; this call consumes dlsym's diagnostic state.
        let symbol_error = unsafe { dlerror() };
        if !symbol_error.is_null() || symbol.is_null() {
            let message = if symbol_error.is_null() {
                "LauncherMain resolved to null".to_owned()
            } else {
                // SAFETY: a non-null `dlerror` result is a NUL-terminated loader string.
                unsafe { CStr::from_ptr(symbol_error) }
                    .to_string_lossy()
                    .into_owned()
            };
            return Err(format!("failed to resolve LauncherMain: {message}"));
        }

        let arguments = env::args_os().collect::<Vec<_>>();
        let argument_count = c_int::try_from(arguments.len())
            .map_err(|_| "argument count exceeds the C launcher limit".to_owned())?;
        let (_owned_arguments, mut argument_pointers) = c_arguments(arguments)?;

        println!(
            "Rust process entry active: {} arguments, {}",
            argument_count,
            launcher_path.display()
        );
        io::stdout()
            .flush()
            .map_err(|error| format!("failed to flush the process-entry marker: {error}"))?;

        // SAFETY: `LauncherMain` is the documented exported symbol with this signature.
        // CString backing storage, argv pointers, and the deliberately retained library handle
        // all remain live until the callback returns.
        let launcher_main: LegacyLauncherMain = unsafe { std::mem::transmute(symbol) };
        Ok(unsafe { launcher_main(argument_count, argument_pointers.as_mut_ptr()) })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::ffi::OsStringExt;

        #[test]
        fn resolves_the_installed_launcher_below_the_executable() {
            let path = installed_launcher_path(Path::new("/tmp/source/hl2_launcher")).unwrap();
            assert_eq!(path, Path::new("/tmp/source/bin").join(library_filename()));
        }

        #[test]
        fn owns_a_null_terminated_exact_argument_vector() {
            let arguments = vec![
                OsString::from_vec(b"hl2_launcher".to_vec()),
                OsString::from_vec(b"-game".to_vec()),
                OsString::from_vec(b"hl2".to_vec()),
            ];
            let (owned, pointers) = c_arguments(arguments).unwrap();
            assert_eq!(owned.len(), 3);
            assert_eq!(owned[1].as_bytes(), b"-game");
            assert_eq!(pointers.len(), 4);
            assert!(pointers[3].is_null());
        }

        #[test]
        fn rejects_interior_nul_arguments() {
            let argument = OsString::from_vec(b"bad\0argument".to_vec());
            assert!(c_arguments(vec![argument]).is_err());
        }
    }
}

#[cfg(unix)]
fn main() {
    match unix::run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("Rust process entry failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("the Rust-owned process entry currently targets macOS/Linux");
    std::process::exit(1);
}
