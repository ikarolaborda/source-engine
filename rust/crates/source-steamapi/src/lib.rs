//! The engine's stub Steamworks library, with no C++ in it.
//!
//! `stub_steam/steam_api.cpp` exists because eight subprojects link `steam_api`
//! and include the real Steamworks headers, while this tree ships no Steamworks
//! implementation. Every entry point is a stub that returns a constant, so the
//! module carries no state and no behaviour: reproducing it means reproducing
//! its exports and their constants exactly, and nothing else.
//!
//! Two things make it unlike the modules ported before it, both of which the
//! engine finds with `CreateInterface` and loads with `dlopen`:
//!
//! * There is no interface and no vtable. The exports are flat C, so this is the
//!   first module here that needs no hand-laid Itanium table at all.
//! * It is a link-time dependency. `libengine` names twenty of these symbols,
//!   `libclient` nine and `libserver` eight, and the linker resolves them
//!   against this library while building. Dropping the C++ subproject therefore
//!   has to leave something behind for `use = ['steam_api']` to resolve to,
//!   which `rust/wscript` does.
//!
//! The C++ declares every function with an empty parameter list while callers
//! call them through the real Steamworks prototypes, which take arguments. That
//! is only well-defined because the caller lays the arguments down and the
//! callee never looks at them; the functions here take no parameters for the
//! same reason, and the differential gate calls both modules through the real
//! prototypes to check that it holds.

use core::ffi::{c_char, c_int, c_void};
use core::ptr;

/// The module's one exported datum. `libengine` and `libserver` both import it,
/// and the C++ leaves it a tentative definition in `__DATA,__common` that
/// nothing ever assigns; a consumer that writes to it must see the write through
/// this storage, which is what the gate checks.
#[no_mangle]
pub static mut g_pSteamClientGameServer: *mut c_void = ptr::null_mut();

/// Writes out the stubs, so that what each one answers is the only thing on the
/// page. `()` covers the functions the C++ declares as `void`.
macro_rules! stubs {
    ($($(#[$attr:meta])* $name:ident() -> $ret:ty = $answer:expr;)*) => {
        $(
            $(#[$attr])*
            #[no_mangle]
            pub extern "C" fn $name() -> $ret {
                $answer
            }
        )*
    };
}

stubs! {
    // steam_api.h
    SteamAPI_Init() -> bool = true;
    SteamAPI_InitSafe() -> bool = true;
    SteamAPI_Shutdown() -> () = ();
    SteamAPI_RestartAppIfNecessary() -> bool = false;
    SteamAPI_ReleaseCurrentThreadMemory() -> () = ();
    SteamAPI_WriteMiniDump() -> () = ();
    SteamAPI_SetMiniDumpComment() -> () = ();
    SteamAPI_RunCallbacks() -> () = ();
    SteamAPI_RegisterCallback() -> () = ();
    SteamAPI_UnregisterCallback() -> () = ();
    SteamAPI_RegisterCallResult() -> () = ();
    SteamAPI_UnregisterCallResult() -> () = ();
    SteamAPI_IsSteamRunning() -> bool = false;

    Steam_RunCallbacks() -> () = ();
    Steam_RegisterInterfaceFuncs() -> () = ();
    Steam_GetHSteamUserCurrent() -> c_int = 0;

    SteamAPI_GetSteamInstallPath() -> *const c_char = ptr::null();
    SteamAPI_GetHSteamPipe() -> c_int = 0;
    SteamAPI_SetTryCatchCallbacks() -> () = ();
    SteamAPI_SetBreakpadAppID() -> () = ();
    SteamAPI_UseBreakpadCrashHandler() -> () = ();

    GetHSteamPipe() -> c_int = 0;
    GetHSteamUser() -> c_int = 0;
    SteamAPI_GetHSteamUser() -> c_int = 0;

    SteamInternal_ContextInit() -> *mut c_void = ptr::null_mut();
    SteamInternal_CreateInterface() -> *mut c_void = ptr::null_mut();

    // Every accessor answers "no such interface", which is what makes the engine
    // take its no-Steam paths.
    SteamApps() -> *mut c_void = ptr::null_mut();
    SteamClient() -> *mut c_void = ptr::null_mut();
    SteamFriends() -> *mut c_void = ptr::null_mut();
    SteamHTTP() -> *mut c_void = ptr::null_mut();
    SteamMatchmaking() -> *mut c_void = ptr::null_mut();
    SteamMatchmakingServers() -> *mut c_void = ptr::null_mut();
    SteamNetworking() -> *mut c_void = ptr::null_mut();
    SteamRemoteStorage() -> *mut c_void = ptr::null_mut();
    SteamScreenshots() -> *mut c_void = ptr::null_mut();
    SteamUser() -> *mut c_void = ptr::null_mut();
    SteamUserStats() -> *mut c_void = ptr::null_mut();
    SteamUtils() -> *mut c_void = ptr::null_mut();

    // steam_gameserver.h. InitSafe answers 0 here where SteamAPI_InitSafe
    // answers true: the C++ declares this one as int, and the gate holds both to
    // the value their own module returns rather than to the shape of the name.
    SteamGameServer_GetHSteamPipe() -> c_int = 0;
    SteamGameServer_GetHSteamUser() -> c_int = 0;
    SteamGameServer_GetIPCCallCount() -> c_int = 0;
    SteamGameServer_InitSafe() -> c_int = 0;
    SteamGameServer_RunCallbacks() -> () = ();
    SteamGameServer_Shutdown() -> () = ();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_succeeds_and_steam_is_never_running() {
        // The two answers the engine actually branches on: it believes it
        // initialised, and it believes Steam is absent.
        assert!(SteamAPI_Init());
        assert!(SteamAPI_InitSafe());
        assert!(!SteamAPI_IsSteamRunning());
        assert!(!SteamAPI_RestartAppIfNecessary());
    }

    #[test]
    fn every_interface_accessor_answers_null() {
        let accessors: [extern "C" fn() -> *mut c_void; 14] = [
            SteamApps,
            SteamClient,
            SteamFriends,
            SteamHTTP,
            SteamMatchmaking,
            SteamMatchmakingServers,
            SteamNetworking,
            SteamRemoteStorage,
            SteamScreenshots,
            SteamUser,
            SteamUserStats,
            SteamUtils,
            SteamInternal_ContextInit,
            SteamInternal_CreateInterface,
        ];
        for accessor in accessors {
            assert!(accessor().is_null());
        }
        assert!(SteamAPI_GetSteamInstallPath().is_null());
    }

    #[test]
    fn every_handle_is_zero() {
        let handles: [extern "C" fn() -> c_int; 8] = [
            GetHSteamPipe,
            GetHSteamUser,
            SteamAPI_GetHSteamPipe,
            SteamAPI_GetHSteamUser,
            Steam_GetHSteamUserCurrent,
            SteamGameServer_GetHSteamPipe,
            SteamGameServer_GetHSteamUser,
            SteamGameServer_GetIPCCallCount,
        ];
        for handle in handles {
            assert_eq!(handle(), 0);
        }
        // Declared int in the C++, unlike SteamAPI_InitSafe's bool.
        assert_eq!(SteamGameServer_InitSafe(), 0);
    }

    #[test]
    fn the_game_server_pointer_starts_null_and_is_real_storage() {
        // The engine imports this as data, so it has to be storage a consumer
        // can write through, not a value handed back by an accessor.
        unsafe {
            assert!(ptr::addr_of!(g_pSteamClientGameServer).read().is_null());
            let slot = ptr::addr_of_mut!(g_pSteamClientGameServer);
            slot.write(&raw mut g_pSteamClientGameServer as *mut c_void);
            assert!(!slot.read().is_null());
            slot.write(ptr::null_mut());
        }
    }
}
