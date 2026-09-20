// Differential gate for the Rust scenefilecache module: loads the native C++
// module and the Rust one into one process, serves both the same shipped
// scenes.image through the real filesystem module, and requires every
// ISceneFileCache answer to agree. The calls are ordinary C++ virtual calls, so
// a wrong table layout or calling convention fails here rather than in a game.
#include <cassert>
#include <climits>
#include <cstdlib>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <set>
#include <string>
#include <vector>
#include <dlfcn.h>
#include "filesystem.h"
#include "scenefilecache/ISceneFileCache.h"
#include "tier0/icommandline.h"
#include "rust_engine_bridge.h"

// The slot numbers rust/crates/source-cppabi hardcodes, read back from the
// compiler: an Itanium pointer to a virtual member holds its table offset.
template <typename Member> static size_t Slot(Member member)
{
	struct { uintptr_t ptr; ptrdiff_t adj; } raw;
	static_assert(sizeof(raw) == sizeof(member), "Itanium member pointer");
	std::memcpy(&raw, &member, sizeof(raw));
#if defined(__aarch64__) || defined(__arm__)
	assert(raw.adj & 1);
	return raw.ptr / sizeof(void *);
#else
	assert(raw.ptr & 1);
	return (raw.ptr - 1) / sizeof(void *);
#endif
}

static IFileSystem *g_fs = NULL;
static bool g_offerFileSystem = true;
static IAppSystem *g_group[2] = {NULL, NULL};
static size_t g_swept = 0;

static void *NoInterface(const char *, int *) { return NULL; }

// CAppSystemGroup::FindSystem: a name held in the group's dictionary is
// returned directly, and any other is put to every system in the group,
// the one being connected included, before the parent is asked. A module
// that takes a lock in QueryInterface it already holds in Connect hangs here.
static void *Factory(const char *name, int *code)
{
	void *found = NULL;
	if (g_offerFileSystem && !std::strcmp(name, FILESYSTEM_INTERFACE_VERSION))
		found = g_fs;
	for (IAppSystem *system : g_group) {
		if (found || !system) continue;
		++g_swept;
		found = system->QueryInterface(name);
	}
	if (!found)
		found = g_fs->QueryInterface(name);
	if (code)
		*code = found ? IFACE_OK : IFACE_FAILED;
	return found;
}

static ISceneFileCache *Load(const char *path)
{
	void *module = dlopen(path, RTLD_NOW | RTLD_LOCAL);
	if (!module) { std::fprintf(stderr, "%s\n", dlerror()); std::exit(1); }
	auto factory = reinterpret_cast<CreateInterfaceFn>(dlsym(module, "CreateInterface"));
	assert(factory);
	int code = -1;
	assert(!factory("SceneFileCache00", &code) && code == IFACE_FAILED);
	assert(!factory("SceneFileCache0022", &code) && code == IFACE_FAILED);
	auto cache = static_cast<ISceneFileCache *>(factory(SCENE_FILE_CACHE_INTERFACE_VERSION, &code));
	assert(cache && code == IFACE_OK);
	assert(cache == factory(SCENE_FILE_CACHE_INTERFACE_VERSION, NULL));
	// Both modules share a leaf name; the object must live in the one asked for.
	Dl_info info;
	char wanted[PATH_MAX], actual[PATH_MAX];
	assert(dladdr(cache, &info) && realpath(path, wanted) && realpath(info.dli_fname, actual));
	assert(!std::strcmp(wanted, actual));
	return cache;
}

static size_t g_checks = 0;
// Scene indices whose bytes were compared, which is fewer than the names asked
// for: several spellings reach one scene, and most names reach none.
static std::set<int> g_scenes;

#define SAME(native, rust, what) do { ++g_checks; if (!((native) == (rust))) { \
	std::fprintf(stderr, "MISMATCH %s: %s\n", what, context.c_str()); std::exit(1); } } while (0)

// Returns whether the name resolved, so the caller can count coverage.
//
// A leading '!' marks a scene whose compressed stream is corrupt. Both report
// success and the declared size, as the native module ignores the decoder's
// failure, but what the two decoders leave in the destination before they
// notice is not comparable: native a partial decode, Rust nothing.
static bool CompareName(ISceneFileCache *native, ISceneFileCache *rust, const std::string &listed)
{
	const bool corrupt = !listed.empty() && listed[0] == '!';
	const std::string name = corrupt ? listed.substr(1) : listed;
	const std::string context = name;
	const size_t size = native->GetSceneBufferSize(name.c_str());
	SAME(size, rust->GetSceneBufferSize(name.c_str()), "GetSceneBufferSize");

	SceneCachedData_t a = {1, 2, 3}, b = {4, 5, 6};
	const bool cached = native->GetSceneCachedData(name.c_str(), &a);
	SAME(cached, rust->GetSceneCachedData(name.c_str(), &b), "GetSceneCachedData");
	SAME(a.msecs, b.msecs, "msecs");
	SAME(a.numSounds, b.numSounds, "numSounds");
	SAME(a.sceneId, b.sceneId, "sceneId");

	// Whole, truncated and oversized destinations. Guard bytes catch a write
	// past the size the caller gave.
	const size_t lengths[] = {size, size / 2, size + 64, 1};
	for (size_t length : lengths) {
		if (!length) continue;
		std::vector<unsigned char> x(length + 8, 0xA5), y(length + 8, 0xA5);
		const bool got = native->GetSceneData(name.c_str(), x.data(), length);
		SAME(got, rust->GetSceneData(name.c_str(), y.data(), length), "GetSceneData");
		if (!corrupt) SAME(x, y, "scene bytes");
		// Neither may write past the length it was given.
		for (size_t guard = length; guard < length + 8; ++guard)
			SAME(x[guard] == 0xA5, y[guard] == 0xA5, "guard bytes");
	}
	if (cached) {
		g_scenes.insert(a.sceneId);
		for (int sound = -1; sound <= a.numSounds; ++sound)
			SAME(native->GetSceneCachedSound(a.sceneId, sound),
				rust->GetSceneCachedSound(a.sceneId, sound), "GetSceneCachedSound");
	}
	return cached;
}

static void CompareAll(ISceneFileCache *native, ISceneFileCache *rust,
	const std::vector<std::string> &names, size_t minimumHits, const char *phase)
{
	size_t hits = 0;
	for (const std::string &name : names)
		hits += CompareName(native, rust, name);

	std::string context = "by index";
	size_t strings = 0;
	for (int id = -2; id < 32767; ++id) {
		const char *x = native->GetSceneString((short)id);
		const char *y = rust->GetSceneString((short)id);
		SAME(!x, !y, "GetSceneString presence");
		if (x) { SAME(std::string(x), std::string(y), "GetSceneString"); ++strings; }
		else if (id > 0) break;
	}
	for (int id : {-32768, -1, 32767})
		SAME(native->GetSceneString((short)id) == NULL, rust->GetSceneString((short)id) == NULL, "GetSceneString limits");
	size_t scenes = 0;
	for (int scene = -2; scene < 1 << 20; ++scene) {
		const short first = native->GetSceneCachedSound(scene, 0);
		SAME(first, rust->GetSceneCachedSound(scene, 0), "GetSceneCachedSound by index");
		SAME(native->GetSceneCachedSound(scene, 1 << 20), rust->GetSceneCachedSound(scene, 1 << 20), "sound range");
		// A scene with no sounds also answers -1, so the end of the directory
		// is where a run of them outlasts any real scene list's gaps.
		if (scene >= 0 && first == -1) { if (++scenes > 4096) break; } else if (scene >= 0) scenes = 0;
	}
	std::printf("%s: %zu names (%zu resolved), %zu strings, %zu checks so far\n",
		phase, names.size(), hits, strings, g_checks);
	if (hits < minimumHits) {
		std::fprintf(stderr, "only %zu of the required %zu names resolved\n", hits, minimumHits);
		std::exit(1);
	}
}

// With a scene image that is not one, the engine's fatal error has to fire:
// the caller expects this process to end inside Init, not to return from it.
static int BadImage(char **argv)
{
	CommandLine()->CreateCmdLine("scenefilecache_differential -nomessagebox");
	void *module = dlopen(argv[2], RTLD_NOW | RTLD_LOCAL);
	assert(module);
	auto factory = reinterpret_cast<CreateInterfaceFn>(dlsym(module, "CreateInterface"));
	g_fs = static_cast<IFileSystem *>(factory(FILESYSTEM_INTERFACE_VERSION, NULL));
	assert(g_fs && g_fs->Connect(NoInterface) && g_fs->Init() == INIT_OK);
	SourceAbiContextConfig config = {sizeof(config), SOURCE_ABI_VERSION, NULL, NULL};
	SourceAbiHandle context = 0;
	assert(source_context_create(&config, &context) == SOURCE_ABI_OK);
	assert(source_rust_bridge_activate(context) == SOURCE_ABI_OK);
	g_fs->AddSearchPath(argv[3], "GAME", PATH_ADD_TO_TAIL);
	ISceneFileCache *cache = Load(argv[4]);
	assert(cache->Connect(Factory));
	cache->Init();
	std::printf("SURVIVED a bad scene image\n");
	return 0;
}

int main(int argc, char **argv)
{
	if (argc == 5 && !std::strcmp(argv[1], "bad-image"))
		return BadImage(argv);
	if (argc != 7) {
		std::fprintf(stderr, "usage: %s <filesystem-module> <game-dir> <native-module> <rust-module> <names-file> <minimum-hits>\n", argv[0]);
		return 2;
	}
	assert(Slot(&IAppSystem::Connect) == 0 && Slot(&IAppSystem::Shutdown) == 4);
	assert(Slot(&ISceneFileCache::GetSceneBufferSize) == 5 && Slot(&ISceneFileCache::GetSceneData) == 6);
	assert(Slot(&ISceneFileCache::GetSceneCachedData) == 7 && Slot(&ISceneFileCache::GetSceneCachedSound) == 8);
	assert(Slot(&ISceneFileCache::GetSceneString) == 9 && Slot(&ISceneFileCache::Reload) == 10);
	assert(Slot(&IBaseFileSystem::Read) == 0 && Slot(&IBaseFileSystem::Open) == 2);
	assert(Slot(&IBaseFileSystem::Close) == 3);
	assert(Slot(static_cast<unsigned int (IBaseFileSystem::*)(FileHandle_t)>(&IBaseFileSystem::Size)) == 6);
	static_assert(sizeof(SceneCachedData_t) == 12, "SceneCachedData_t layout");

	CommandLine()->CreateCmdLine("scenefilecache_differential -nomessagebox");
	void *module = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
	if (!module) { std::fprintf(stderr, "%s\n", dlerror()); return 1; }
	auto factory = reinterpret_cast<CreateInterfaceFn>(dlsym(module, "CreateInterface"));
	assert(factory);
	g_fs = static_cast<IFileSystem *>(factory(FILESYSTEM_INTERFACE_VERSION, NULL));
	assert(g_fs && g_fs->Connect(NoInterface) && g_fs->Init() == INIT_OK);
	// Where the Rust binding finds IBaseFileSystem when only IFileSystem is offered.
	assert((char *)static_cast<IBaseFileSystem *>(g_fs) - (char *)g_fs == (ptrdiff_t)sizeof(void *));
	SourceAbiContextConfig config = {sizeof(config), SOURCE_ABI_VERSION, NULL, NULL};
	SourceAbiHandle context = 0;
	assert(source_context_create(&config, &context) == SOURCE_ABI_OK);
	assert(source_rust_bridge_activate(context) == SOURCE_ABI_OK);

	std::vector<std::string> names;
	std::ifstream list(argv[5]);
	for (std::string line; std::getline(list, line);)
		if (!line.empty()) names.push_back(line);
	// Shapes no shipped script uses: every normalization branch and both
	// sides of the native 260-byte name buffer.
	const size_t shipped = names.size();
	for (size_t i = 0; i < shipped && i < 64; ++i) {
		std::string name = names[i], upper = name, back = name, bare = name, other = name;
		for (char &c : upper) c = (char)toupper((unsigned char)c);
		for (char &c : back) if (c == '/') c = '\\';
		bare.resize(bare.size() - 4);
		other.replace(other.size() - 3, 3, "txt");
		for (const std::string &shape : {upper, back, bare, other, bare + ".", name + ".vcd", "./" + name, name + "/"})
			names.push_back(shape);
	}
	for (const char *odd : {"", ".", "..", ".vcd", "/", "\\", "scenes", "scenes/", "a.b.c", ".hidden", "scenes/no_such_scene.vcd"})
		names.push_back(odd);
	for (size_t length : {254u, 255u, 256u, 257u, 258u, 259u, 260u, 261u, 400u}) {
		names.push_back(std::string(length, 'x'));
		names.push_back("scenes/" + std::string(length - 7, 'y') + ".vcd");
	}

	ISceneFileCache *native = Load(argv[3]);
	ISceneFileCache *rust = Load(argv[4]);
	assert(native != rust);
	const size_t minimum = (size_t)std::strtoul(argv[6], NULL, 10);

	g_group[0] = native;
	g_group[1] = rust;

	// Before any image is mounted, both must answer as empty.
	assert(native->Connect(Factory) && rust->Connect(Factory));
	assert(!rust->QueryInterface(SCENE_FILE_CACHE_INTERFACE_VERSION));
	CompareAll(native, rust, names, 0, "unmounted");
	assert(native->Init() == INIT_OK && rust->Init() == INIT_OK);
	CompareAll(native, rust, names, 0, "no image on the search path");

	g_fs->AddSearchPath(argv[2], "GAME", PATH_ADD_TO_TAIL);
	native->Reload();
	rust->Reload();
	CompareAll(native, rust, names, minimum, "mounted");

	// A second Init keeps the resident image, and Shutdown drops it.
	assert(native->Init() == INIT_OK && rust->Init() == INIT_OK);
	CompareAll(native, rust, names, minimum, "second Init");
	native->Shutdown();
	rust->Shutdown();
	CompareAll(native, rust, names, 0, "after Shutdown");

	// The engine rebuilds the group for every mod without unloading the
	// module, so the whole lifecycle has to work a second time. This time the
	// group does not hold the filesystem under its own name, which leaves the
	// Rust module the sweep for IBaseFileSystem.
	native->Disconnect();
	rust->Disconnect();
	g_offerFileSystem = false;
	const size_t sweptBefore = g_swept;
	assert(rust->Connect(Factory));
	assert(g_swept > sweptBefore);
	g_offerFileSystem = true;
	assert(native->Connect(Factory));
	assert(native->Init() == INIT_OK && rust->Init() == INIT_OK);
	CompareAll(native, rust, names, minimum, "second lifecycle, base interface by sweep");
	native->Shutdown();
	rust->Shutdown();
	native->Disconnect();
	rust->Disconnect();

	// The image's own count, to say how much of it the names reached.
	int header[3] = {0, 0, 0};
	std::ifstream image(std::string(argv[2]) + "/scenes/scenes.image", std::ios::binary);
	image.read(reinterpret_cast<char *>(header), sizeof(header));
	std::printf("PASS: %zu comparisons, native and Rust scenefilecache agree; bytes compared for %zu of %d scenes\n",
		g_checks, g_scenes.size(), header[2]);
	return 0;
}
