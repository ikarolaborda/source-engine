// Differential gate for the Rust soundemittersystem module: loads the C++
// module it replaces and the Rust one into one process, gives both the same
// shipped sound scripts through the real filesystem module, and requires the
// same answers for every sound either of them loaded.
//
// Indices are each module's own, so everything is compared by sound name.
// Values that are deliberately random per call (the volume, pitch and sound
// level actually drawn, and which wave of a set is picked) are compared as
// the ranges they are drawn from.
#include <cassert>
#include <climits>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <set>
#include <string>
#include <vector>
#include <dlfcn.h>
#include "filesystem.h"
#include "SoundEmitterSystem/isoundemittersystembase.h"
#include "tier0/icommandline.h"
#include "rust_engine_bridge.h"

static IFileSystem *g_fs = NULL;
static IAppSystem *g_group[2] = {NULL, NULL};

static void *NoInterface(const char *, int *) { return NULL; }

// CAppSystemGroup::FindSystem, which puts a name it does not hold to every
// system in the group, including the one still inside Connect.
static void *Factory(const char *name, int *code)
{
	void *found = NULL;
	if (!std::strcmp(name, FILESYSTEM_INTERFACE_VERSION))
		found = g_fs;
	for (IAppSystem *system : g_group) {
		if (found || !system) continue;
		found = system->QueryInterface(name);
	}
	if (code)
		*code = found ? IFACE_OK : IFACE_FAILED;
	return found;
}

static ISoundEmitterSystemBase *Load(const char *path)
{
	void *module = dlopen(path, RTLD_NOW | RTLD_LOCAL);
	if (!module) { std::fprintf(stderr, "%s\n", dlerror()); std::exit(1); }
	auto factory = reinterpret_cast<CreateInterfaceFn>(dlsym(module, "CreateInterface"));
	assert(factory);
	int code = -1;
	assert(!factory("VSoundEmitter001", &code) && code == IFACE_FAILED);
	auto system = static_cast<ISoundEmitterSystemBase *>(
		factory(SOUNDEMITTERSYSTEM_INTERFACE_VERSION, &code));
	assert(system && code == IFACE_OK);
	Dl_info info;
	char wanted[PATH_MAX], actual[PATH_MAX];
	assert(dladdr(system, &info) && realpath(path, wanted) && realpath(info.dli_fname, actual));
	assert(!std::strcmp(wanted, actual));
	return system;
}

// Everything about one sound that does not change from call to call.
struct Entry
{
	int channel = 0;
	int soundLevelStart = 0, soundLevelRange = 0;
	int pitchStart = 0, pitchRange = 0;
	unsigned short volumeStart = 0, volumeRange = 0;
	int delay = 0;
	bool ownerOnly = false, gendered = false, preload = false;
	std::string script;
	std::vector<std::string> waves;
	std::vector<std::string> converted;

	bool operator==(const Entry &other) const
	{
		return channel == other.channel
			&& soundLevelStart == other.soundLevelStart && soundLevelRange == other.soundLevelRange
			&& pitchStart == other.pitchStart && pitchRange == other.pitchRange
			&& volumeStart == other.volumeStart && volumeRange == other.volumeRange
			&& delay == other.delay && ownerOnly == other.ownerOnly
			&& gendered == other.gendered && preload == other.preload
			&& script == other.script && waves == other.waves && converted == other.converted;
	}
};

static std::string Describe(const Entry &e)
{
	char text[512];
	std::string waves;
	for (const std::string &wave : e.waves) waves += wave + " ";
	std::snprintf(text, sizeof(text),
		"channel %d level %d+%d pitch %d+%d volume %04x+%04x delay %d owner %d gender %d preload %d script %s waves %zu %s",
		e.channel, e.soundLevelStart, e.soundLevelRange, e.pitchStart, e.pitchRange,
		e.volumeStart, e.volumeRange, e.delay, e.ownerOnly, e.gendered, e.preload,
		e.script.c_str(), e.waves.size(), waves.c_str());
	return text;
}

static std::map<std::string, Entry> Collect(ISoundEmitterSystemBase *system)
{
	std::map<std::string, Entry> entries;
	const int count = system->GetSoundCount();
	for (int index = 0; index < count; ++index) {
		const char *name = system->GetSoundName(index);
		if (!name || !name[0]) continue;
		CSoundParametersInternal *params = system->InternalGetParametersForSound(index);
		if (!params) { std::fprintf(stderr, "no params for %s\n", name); std::exit(1); }

		Entry entry;
		entry.channel = params->GetChannel();
		entry.soundLevelStart = params->GetSoundLevel().start;
		entry.soundLevelRange = params->GetSoundLevel().range;
		entry.pitchStart = params->GetPitch().start;
		entry.pitchRange = params->GetPitch().range;
		entry.volumeStart = params->GetVolume().start.GetBits();
		entry.volumeRange = params->GetVolume().range.GetBits();
		entry.delay = params->GetDelayMsec();
		entry.ownerOnly = params->OnlyPlayToOwner();
		entry.gendered = params->UsesGenderToken();
		entry.preload = params->ShouldPreload();
		const char *script = system->GetSourceFileForSound(index);
		entry.script = script ? script : "";
		for (int wave = 0; wave < params->NumSoundNames(); ++wave) {
			CUtlSymbol symbol = params->GetSoundNames()[wave].symbol;
			const char *wavename = system->GetWaveName(symbol);
			char shaped[512];
			std::snprintf(shaped, sizeof(shaped), "%s|%d",
				wavename ? wavename : "", params->GetSoundNames()[wave].gender);
			entry.waves.push_back(shaped);
		}
		for (int wave = 0; wave < params->NumConvertedNames(); ++wave) {
			CUtlSymbol symbol = params->GetConvertedNames()[wave].symbol;
			const char *wavename = system->GetWaveName(symbol);
			entry.converted.push_back(wavename ? wavename : "");
		}
		entries[name] = entry;
	}
	return entries;
}

static size_t g_checks = 0;

#define SAME(a, b, what, context) do { ++g_checks; if (!((a) == (b))) { \
	std::fprintf(stderr, "MISMATCH %s for %s\n", what, std::string(context).c_str()); std::exit(1); } } while (0)

int main(int argc, char **argv)
{
	if (argc != 5) {
		std::fprintf(stderr, "usage: %s <filesystem-module> <game-dir>[:<game-dir>...] <native-module> <rust-module>\n", argv[0]);
		return 2;
	}
	static_assert(sizeof(CSoundParametersInternal) == 36, "resident parameter layout");
	static_assert(sizeof(SoundFile) == 4, "wave slot layout");

	CommandLine()->CreateCmdLine("soundemitter_differential -nomessagebox");
	void *module = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
	if (!module) { std::fprintf(stderr, "%s\n", dlerror()); return 1; }
	auto factory = reinterpret_cast<CreateInterfaceFn>(dlsym(module, "CreateInterface"));
	assert(factory);
	g_fs = static_cast<IFileSystem *>(factory(FILESYSTEM_INTERFACE_VERSION, NULL));
	assert(g_fs && g_fs->Connect(NoInterface) && g_fs->Init() == INIT_OK);
	SourceAbiContextConfig config = {sizeof(config), SOURCE_ABI_VERSION, NULL, NULL};
	SourceAbiHandle context = 0;
	assert(source_context_create(&config, &context) == SOURCE_ABI_OK);
	assert(source_rust_bridge_activate(context) == SOURCE_ABI_OK);
	// A mod's own content first, then what it inherits, which is the order
	// its gameinfo.txt mounts them in.
	for (std::string paths = argv[2]; !paths.empty();) {
		const size_t split = paths.find(':');
		g_fs->AddSearchPath(paths.substr(0, split).c_str(), "GAME", PATH_ADD_TO_TAIL);
		paths = split == std::string::npos ? "" : paths.substr(split + 1);
	}

	ISoundEmitterSystemBase *native = Load(argv[3]);
	ISoundEmitterSystemBase *rust = Load(argv[4]);
	g_group[0] = native;
	g_group[1] = rust;
	assert(native->Connect(Factory) && rust->Connect(Factory));
	assert(native->Init() == INIT_OK && rust->Init() == INIT_OK);
	// Nothing is loaded until the mod is known.
	SAME(native->GetSoundCount(), rust->GetSoundCount(), "sound count before ModInit", "startup");
	assert(native->ModInit() && rust->ModInit());

	const std::map<std::string, Entry> nativeEntries = Collect(native);
	const std::map<std::string, Entry> rustEntries = Collect(rust);
	SAME(nativeEntries.size(), rustEntries.size(), "number of sounds", "all");
	SAME(native->GetNumSoundScripts(), rust->GetNumSoundScripts(), "number of scripts", "all");
	SAME(native->GetManifestFileTimeChecksum(), rust->GetManifestFileTimeChecksum(), "manifest checksum", "all");

	size_t gendered = 0, multiwave = 0;
	for (const auto &pair : nativeEntries) {
		const std::string &name = pair.first;
		auto found = rustEntries.find(name);
		if (found == rustEntries.end()) {
			std::fprintf(stderr, "MISSING in Rust module: %s\n", name.c_str());
			return 1;
		}
		if (!(pair.second == found->second)) {
			std::fprintf(stderr, "MISMATCH for %s\n  native: %s\n  rust:   %s\n",
				name.c_str(), Describe(pair.second).c_str(), Describe(found->second).c_str());
			return 1;
		}
		++g_checks;
		gendered += pair.second.gendered;
		multiwave += pair.second.waves.size() > 1;

		// The index each module gives the name must lead back to that name.
		const int nativeIndex = native->GetSoundIndex(name.c_str());
		const int rustIndex = rust->GetSoundIndex(name.c_str());
		SAME(true, native->IsValidIndex(nativeIndex), "native index valid", name);
		SAME(true, rust->IsValidIndex(rustIndex), "rust index valid", name);
		SAME(std::string(native->GetSoundName(nativeIndex)), name, "native index round trip", name);
		SAME(std::string(rust->GetSoundName(rustIndex)), name, "rust index round trip", name);
		SAME(native->IsUsingGenderToken(name.c_str()), rust->IsUsingGenderToken(name.c_str()),
			"gender token", name);

		// One play through each interface: the fields that do not vary, and
		// a wave that belongs to the entry.
		CSoundParameters a, b;
		const bool gotA = native->GetParametersForSound(name.c_str(), a, GENDER_NONE);
		const bool gotB = rust->GetParametersForSound(name.c_str(), b, GENDER_NONE);
		SAME(gotA, gotB, "GetParametersForSound", name);
		if (gotA) {
			SAME(a.channel, b.channel, "channel", name);
			SAME(a.count, b.count, "wave count", name);
			SAME(a.delay_msec, b.delay_msec, "delay", name);
			SAME(a.play_to_owner_only, b.play_to_owner_only, "owner only", name);
			SAME(a.pitchlow, b.pitchlow, "pitch low", name);
			SAME(a.pitchhigh, b.pitchhigh, "pitch high", name);
			if (pair.second.soundLevelRange == 0)
				SAME(a.soundlevel, b.soundlevel, "sound level", name);
			std::string chosen = std::string(b.soundname) + "|";
			bool belongs = false;
			for (const std::string &wave : found->second.waves)
				belongs = belongs || wave.compare(0, chosen.size(), chosen) == 0;
			SAME(true, belongs, "chosen wave belongs to the entry", name);
		}
	}

	for (const auto &pair : rustEntries) {
		if (nativeEntries.count(pair.first)) continue;
		std::fprintf(stderr, "EXTRA in Rust module: %s\n", pair.first.c_str());
		return 1;
	}

	// Speakers and the paths their lines come from.
	const char *models[] = {"models/alyx.mdl", "models/barney.mdl", "models/police.mdl",
		"models/humans/group01/male_01.mdl", "", "models/not_an_actor.mdl"};
	for (const char *model : models) {
		SAME(native->GetActorGender(model), rust->GetActorGender(model), "actor gender", model);
		for (const char *line : {"vo/npc/$gender01/hello.wav", "ambient/wind.wav", "$GENDER", ""}) {
			char one[256] = {0}, two[256] = {0};
			native->GenderExpandString(model, line, one, sizeof(one));
			rust->GenderExpandString(model, line, two, sizeof(two));
			SAME(std::string(one), std::string(two), "gender expansion", std::string(model) + " " + line);
		}
	}
	for (int gender = GENDER_NONE; gender <= GENDER_FEMALE; ++gender) {
		char one[256] = {0}, two[256] = {0};
		native->GenderExpandString((gender_t)gender, "vo/$gender01/a.wav", one, sizeof(one));
		rust->GenderExpandString((gender_t)gender, "vo/$gender01/a.wav", two, sizeof(two));
		SAME(std::string(one), std::string(two), "gender expansion by gender", "explicit");
	}

	// Names that are not sounds, and edges around the index space.
	for (const char *missing : {"", "no.such.sound", "Weapon", "weapon.fire "}) {
		SAME(native->GetSoundIndex(missing), rust->GetSoundIndex(missing), "missing sound index", missing);
		CSoundParameters a, b;
		SAME(native->GetParametersForSound(missing, a, GENDER_NONE),
			rust->GetParametersForSound(missing, b, GENDER_NONE), "missing sound params", missing);
		SAME(native->IsUsingGenderToken(missing), rust->IsUsingGenderToken(missing), "missing gender token", missing);
	}
	for (int index : {-1, 0, (int)nativeEntries.size(), (int)nativeEntries.size() + 1, 1 << 20}) {
		const bool nativeValid = native->IsValidIndex(index);
		SAME(nativeValid, rust->IsValidIndex(index), "index validity", std::to_string(index));
		if (!nativeValid) {
			SAME(true, rust->InternalGetParametersForSound(index) == NULL, "no params off the end",
				std::to_string(index));
			SAME(std::string(rust->GetSoundName(index)), std::string(""), "no name off the end",
				std::to_string(index));
		}
	}

	// Walking the whole table must reach every sound exactly once.
	std::set<std::string> walked;
	for (int index = rust->First(); index != rust->InvalidIndex(); index = rust->Next(index)) {
		const char *name = rust->GetSoundName(index);
		SAME(true, walked.insert(name ? name : "").second, "walk visits a sound once",
			name ? name : "");
	}
	SAME(walked.size(), rustEntries.size(), "walk covers every sound", "all");

	// A map's overrides replace entries while they are loaded, and clearing
	// them puts the originals back.
	const size_t before = rustEntries.size();
	rust->ClearSoundOverrides();
	SAME(before, (size_t)rust->GetSoundCount(), "clearing nothing changes nothing", "overrides");
	rust->Flush();
	SAME(before, (size_t)rust->GetSoundCount(), "reloading gives the same sounds", "flush");

	std::printf("PASS: %zu comparisons over %zu sounds (%zu gendered, %zu with several waves), %d scripts\n",
		g_checks, nativeEntries.size(), gendered, multiwave, native->GetNumSoundScripts());
	native->ModShutdown();
	rust->ModShutdown();
	return 0;
}
