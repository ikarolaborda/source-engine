// Link-and-call gate for the steam_api module.
//
// The differential harness beside this one opens both modules with dlopen,
// because they export the same names. That answers what each export returns but
// not the two things only linking can answer, and linking is how all eight
// subprojects that name `steam_api` actually use it:
//
//   * Whether the module still resolves at run time. The C++ module recorded an
//     absolute build path in every consumer; Cargo records @rpath, and nothing
//     in this tree carries an LC_RPATH. What makes both work is the launcher
//     putting the game's bin directory on DYLD_LIBRARY_PATH, which dyld searches
//     by leaf name first. This program is linked the same way and run the same
//     way, so it fails if that stops being true.
//
//   * Whether calling through the real Steamworks prototypes is safe. The module
//     declares every function with an empty parameter list while callers declare
//     and call them with arguments, which is only well-defined because the callee
//     never reads them. The declarations below are the ones from
//     public/steam/steam_api.h, so the calls lay down arguments exactly as the
//     engine does.
//
// The transcript is compared between the two modules by the shell script; it is
// printed rather than asserted so that a difference shows what changed.

#include <stdint.h>
#include <stdio.h>

extern "C" {

// Taken from public/steam/steam_api.h and steam_gameserver.h. Pointers to
// CCallbackBase are declared void * here: the module never dereferences them,
// and a pointer is a pointer to the ABI.
bool SteamAPI_Init();
bool SteamAPI_InitSafe();
void SteamAPI_Shutdown();
bool SteamAPI_RestartAppIfNecessary();
void SteamAPI_ReleaseCurrentThreadMemory();
void SteamAPI_WriteMiniDump(uint32_t uStructuredExceptionCode, void *pvExceptionInfo, uint32_t uBuildID);
void SteamAPI_SetMiniDumpComment(const char *pchMsg);
void SteamAPI_RunCallbacks();
void SteamAPI_RegisterCallback(void *pCallback, int iCallback);
void SteamAPI_UnregisterCallback(void *pCallback);
void SteamAPI_RegisterCallResult(void *pCallback, uint64_t hAPICall);
void SteamAPI_UnregisterCallResult(void *pCallback, uint64_t hAPICall);
bool SteamAPI_IsSteamRunning();
void Steam_RunCallbacks();
void Steam_RegisterInterfaceFuncs();
int Steam_GetHSteamUserCurrent();
const char *SteamAPI_GetSteamInstallPath();
int SteamAPI_GetHSteamPipe();
void SteamAPI_SetTryCatchCallbacks(bool bTryCatchCallbacks);
void SteamAPI_SetBreakpadAppID(uint32_t unAppID);
void SteamAPI_UseBreakpadCrashHandler(const char *pchVersion, const char *pchDate,
	const char *pchTime, bool bFullMemoryDumps, void *pvContext, void *pfnPreMinidumpCallback);
int GetHSteamPipe();
int GetHSteamUser();
int SteamAPI_GetHSteamUser();
void *SteamInternal_ContextInit(void *pContextInitData);
void *SteamInternal_CreateInterface(const char *ver);
void *SteamApps();
void *SteamClient();
void *SteamFriends();
void *SteamHTTP();
void *SteamMatchmaking();
void *SteamMatchmakingServers();
void *SteamNetworking();
void *SteamRemoteStorage();
void *SteamScreenshots();
void *SteamUser();
void *SteamUserStats();
void *SteamUtils();
int SteamGameServer_GetHSteamPipe();
int SteamGameServer_GetHSteamUser();
int SteamGameServer_GetIPCCallCount();
int SteamGameServer_InitSafe();
void SteamGameServer_RunCallbacks();
void SteamGameServer_Shutdown();

extern void *g_pSteamClientGameServer;

} // extern "C"

namespace {

void report(const char *name, long value)
{
	printf("%s = %ld\n", name, value);
}

void report_pointer(const char *name, const void *value)
{
	// Addresses differ between the two modules; only nullness is contractual.
	printf("%s = %s\n", name, value ? "non-null" : "null");
}

} // namespace

int main()
{
	// Calls that take arguments, made the way the engine makes them.
	char callback[64] = {0};
	SteamAPI_WriteMiniDump(0xC0000005u, callback, 1234u);
	SteamAPI_SetMiniDumpComment("steam_api gate");
	SteamAPI_RegisterCallback(callback, 101);
	SteamAPI_UnregisterCallback(callback);
	SteamAPI_RegisterCallResult(callback, 0x1122334455667788ull);
	SteamAPI_UnregisterCallResult(callback, 0x1122334455667788ull);
	SteamAPI_SetTryCatchCallbacks(true);
	SteamAPI_SetBreakpadAppID(220u);
	SteamAPI_UseBreakpadCrashHandler("1.0", __DATE__, __TIME__, true, callback, NULL);
	SteamAPI_ReleaseCurrentThreadMemory();
	SteamAPI_RunCallbacks();
	SteamAPI_Shutdown();
	Steam_RunCallbacks();
	Steam_RegisterInterfaceFuncs();
	SteamGameServer_RunCallbacks();
	SteamGameServer_Shutdown();
	printf("argument-taking calls returned\n");

	report("SteamAPI_Init", SteamAPI_Init());
	report("SteamAPI_InitSafe", SteamAPI_InitSafe());
	report("SteamAPI_RestartAppIfNecessary", SteamAPI_RestartAppIfNecessary());
	report("SteamAPI_IsSteamRunning", SteamAPI_IsSteamRunning());
	report("Steam_GetHSteamUserCurrent", Steam_GetHSteamUserCurrent());
	report("SteamAPI_GetHSteamPipe", SteamAPI_GetHSteamPipe());
	report("SteamAPI_GetHSteamUser", SteamAPI_GetHSteamUser());
	report("GetHSteamPipe", GetHSteamPipe());
	report("GetHSteamUser", GetHSteamUser());
	report("SteamGameServer_GetHSteamPipe", SteamGameServer_GetHSteamPipe());
	report("SteamGameServer_GetHSteamUser", SteamGameServer_GetHSteamUser());
	report("SteamGameServer_GetIPCCallCount", SteamGameServer_GetIPCCallCount());
	report("SteamGameServer_InitSafe", SteamGameServer_InitSafe());

	report_pointer("SteamAPI_GetSteamInstallPath", SteamAPI_GetSteamInstallPath());
	report_pointer("SteamInternal_ContextInit", SteamInternal_ContextInit(callback));
	report_pointer("SteamInternal_CreateInterface", SteamInternal_CreateInterface("SteamClient017"));
	report_pointer("SteamApps", SteamApps());
	report_pointer("SteamClient", SteamClient());
	report_pointer("SteamFriends", SteamFriends());
	report_pointer("SteamHTTP", SteamHTTP());
	report_pointer("SteamMatchmaking", SteamMatchmaking());
	report_pointer("SteamMatchmakingServers", SteamMatchmakingServers());
	report_pointer("SteamNetworking", SteamNetworking());
	report_pointer("SteamRemoteStorage", SteamRemoteStorage());
	report_pointer("SteamScreenshots", SteamScreenshots());
	report_pointer("SteamUser", SteamUser());
	report_pointer("SteamUserStats", SteamUserStats());
	report_pointer("SteamUtils", SteamUtils());

	// The imported datum, reached the way libengine and libserver reach it.
	report_pointer("g_pSteamClientGameServer", g_pSteamClientGameServer);
	g_pSteamClientGameServer = callback;
	report_pointer("g_pSteamClientGameServer after write", g_pSteamClientGameServer);
	printf("g_pSteamClientGameServer holds what was written = %d\n",
		g_pSteamClientGameServer == callback);
	g_pSteamClientGameServer = NULL;
	report_pointer("g_pSteamClientGameServer after clear", g_pSteamClientGameServer);
	return 0;
}
