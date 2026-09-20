// Differential gate for the Rust steam_api module.
//
// The module it replaces is flat C: forty-four functions that each return a
// constant, and one exported datum. There is no interface to drive, so the
// comparison is of every export's answer, taken from both modules in the same
// process and required to be identical.
//
// Both modules export the same forty-five names, so neither can be linked here:
// the first one loaded would answer for both. They are opened with dlopen and
// every symbol is fetched with dlsym, and dladdr is asserted on each one because
// DYLD_LIBRARY_PATH resolves a full path by leaf name first, which would
// otherwise let two differently-named paths quietly be the same library.

#include <dlfcn.h>
#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

namespace {

int g_failures = 0;
int g_comparisons = 0;

void fail(const char *name, const char *detail)
{
	fprintf(stderr, "FAIL %s: %s\n", name, detail);
	++g_failures;
}

struct Module
{
	const char *label;
	char path[PATH_MAX];
	void *handle;
};

void open_module(Module &module, const char *label, const char *path)
{
	module.label = label;
	if (!realpath(path, module.path)) {
		fprintf(stderr, "cannot resolve %s: %s\n", path, strerror(errno));
		exit(2);
	}
	module.handle = dlopen(module.path, RTLD_NOW | RTLD_LOCAL);
	if (!module.handle) {
		fprintf(stderr, "cannot load %s: %s\n", module.path, dlerror());
		exit(2);
	}
}

// A symbol that resolved into some other library would compare equal for the
// wrong reason. Every lookup is held to the library it was asked of.
void *symbol(const Module &module, const char *name)
{
	dlerror();
	void *address = dlsym(module.handle, name);
	if (!address) {
		fprintf(stderr, "%s does not export %s\n", module.path, name);
		exit(2);
	}

	Dl_info info;
	char resolved[PATH_MAX];
	if (!dladdr(address, &info) || !realpath(info.dli_fname, resolved) ||
	    strcmp(resolved, module.path) != 0) {
		fprintf(stderr, "%s resolved into %s, not %s\n", name,
			info.dli_fname ? info.dli_fname : "(unknown)", module.path);
		exit(2);
	}
	return address;
}

template <typename Fn>
Fn function(const Module &module, const char *name)
{
	// The cast dlsym's void* needs; the shapes below are the C++ module's own
	// declarations, which take no parameters.
	Fn address;
	void *raw = symbol(module, name);
	memcpy(&address, &raw, sizeof address);
	return address;
}

const char *const kVoidFunctions[] = {
	"SteamAPI_Shutdown",
	"SteamAPI_ReleaseCurrentThreadMemory",
	"SteamAPI_WriteMiniDump",
	"SteamAPI_SetMiniDumpComment",
	"SteamAPI_RunCallbacks",
	"SteamAPI_RegisterCallback",
	"SteamAPI_UnregisterCallback",
	"SteamAPI_RegisterCallResult",
	"SteamAPI_UnregisterCallResult",
	"SteamAPI_SetTryCatchCallbacks",
	"SteamAPI_SetBreakpadAppID",
	"SteamAPI_UseBreakpadCrashHandler",
	"Steam_RunCallbacks",
	"Steam_RegisterInterfaceFuncs",
	"SteamGameServer_RunCallbacks",
	"SteamGameServer_Shutdown",
};

const char *const kBoolFunctions[] = {
	"SteamAPI_Init",
	"SteamAPI_InitSafe",
	"SteamAPI_RestartAppIfNecessary",
	"SteamAPI_IsSteamRunning",
};

const char *const kIntFunctions[] = {
	"Steam_GetHSteamUserCurrent",
	"SteamAPI_GetHSteamPipe",
	"SteamAPI_GetHSteamUser",
	"GetHSteamPipe",
	"GetHSteamUser",
	"SteamGameServer_GetHSteamPipe",
	"SteamGameServer_GetHSteamUser",
	"SteamGameServer_GetIPCCallCount",
	"SteamGameServer_InitSafe",
};

const char *const kPointerFunctions[] = {
	"SteamInternal_ContextInit",
	"SteamInternal_CreateInterface",
	"SteamApps",
	"SteamClient",
	"SteamFriends",
	"SteamHTTP",
	"SteamMatchmaking",
	"SteamMatchmakingServers",
	"SteamNetworking",
	"SteamRemoteStorage",
	"SteamScreenshots",
	"SteamUser",
	"SteamUserStats",
	"SteamUtils",
};

template <size_t N>
void compare_void(const Module &oracle, const Module &rust, const char *const (&names)[N])
{
	// Nothing to compare but the fact that both return; a module that crashed
	// or hung here would never reach the next line.
	for (size_t i = 0; i < N; ++i) {
		function<void (*)()>(oracle, names[i])();
		function<void (*)()>(rust, names[i])();
		++g_comparisons;
	}
}

template <size_t N>
void compare_bool(const Module &oracle, const Module &rust, const char *const (&names)[N])
{
	for (size_t i = 0; i < N; ++i) {
		bool expected = function<bool (*)()>(oracle, names[i])();
		bool actual = function<bool (*)()>(rust, names[i])();
		if (expected != actual) {
			char detail[128];
			snprintf(detail, sizeof detail, "oracle %d, rust %d", expected, actual);
			fail(names[i], detail);
		}
		++g_comparisons;
	}
}

template <size_t N>
void compare_int(const Module &oracle, const Module &rust, const char *const (&names)[N])
{
	for (size_t i = 0; i < N; ++i) {
		int expected = function<int (*)()>(oracle, names[i])();
		int actual = function<int (*)()>(rust, names[i])();
		if (expected != actual) {
			char detail[128];
			snprintf(detail, sizeof detail, "oracle %d, rust %d", expected, actual);
			fail(names[i], detail);
		}
		++g_comparisons;
	}
}

template <size_t N>
void compare_pointer(const Module &oracle, const Module &rust, const char *const (&names)[N])
{
	for (size_t i = 0; i < N; ++i) {
		void *expected = function<void *(*)()>(oracle, names[i])();
		void *actual = function<void *(*)()>(rust, names[i])();
		if (expected != actual) {
			char detail[128];
			snprintf(detail, sizeof detail, "oracle %p, rust %p", expected, actual);
			fail(names[i], detail);
		}
		++g_comparisons;
	}
}

// libengine and libserver import this as data, not through an accessor, so what
// has to match is the storage: it starts null, and a write through it is
// readable back at the same address.
void compare_game_server_pointer(const Module &module)
{
	void **slot = static_cast<void **>(symbol(module, "g_pSteamClientGameServer"));
	if (*slot != NULL) {
		fail("g_pSteamClientGameServer", "does not start null");
	}
	++g_comparisons;

	void *marker = reinterpret_cast<void *>(slot);
	*slot = marker;
	if (*slot != marker) {
		fail("g_pSteamClientGameServer", "does not hold a written value");
	}
	++g_comparisons;

	*slot = NULL;
}

} // namespace

int main(int argc, char **argv)
{
	if (argc != 3) {
		fprintf(stderr, "usage: %s <native-module> <rust-module>\n", argv[0]);
		return 2;
	}

	Module oracle;
	Module rust;
	open_module(oracle, "native", argv[1]);
	open_module(rust, "rust", argv[2]);
	if (strcmp(oracle.path, rust.path) == 0) {
		fprintf(stderr, "both modules resolved to %s\n", oracle.path);
		return 2;
	}

	compare_void(oracle, rust, kVoidFunctions);
	compare_bool(oracle, rust, kBoolFunctions);
	compare_int(oracle, rust, kIntFunctions);
	compare_pointer(oracle, rust, kPointerFunctions);
	compare_game_server_pointer(oracle);
	compare_game_server_pointer(rust);

	// Repeat the whole set: the C++ module holds no state, and a Rust module
	// that quietly held some would answer differently the second time.
	compare_bool(oracle, rust, kBoolFunctions);
	compare_int(oracle, rust, kIntFunctions);
	compare_pointer(oracle, rust, kPointerFunctions);

	printf("%d comparisons over %d exports, %d failures\n",
		g_comparisons, 45, g_failures);
	return g_failures == 0 ? 0 : 1;
}
