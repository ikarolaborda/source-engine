// Load the actual filesystem module. Exercise its absolute/filtered pack
// adapter, whose index, payload bytes, decoding and cursors are owned by Rust.
#include <cassert>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <iterator>
#include <set>
#include <string>
#include <vector>
#include <utility>
#include <dlfcn.h>
#include "filesystem.h"
#include "tier0/icommandline.h"
#include "rust_engine_bridge.h"

static_assert(sizeof(SourceAbiPakEntry) == 32, "stable C ABI pack entry layout");
static_assert(sizeof(SourceAbiPakArchiveInfo) == 32, "stable C ABI archive discovery layout");
static_assert(sizeof(SourceAbiSearchPath) == 24, "stable search-path descriptor layout");

static SourceAbiSlice Span(const std::string &s)
{
	SourceAbiSlice value = {reinterpret_cast<const uint8_t *>(s.data()), s.size()};
	return value;
}
static void *NoInterface(const char *, int *) { return NULL; }

typedef std::vector<std::pair<std::string, bool> > Matches;
static Matches FindPack(IFileSystem *fs, const char *pattern)
{
	FileFindHandle_t find;
	Matches result;
	const char *name = fs->FindFirstEx(pattern, "BSP", &find);
	if (name) {
		do { result.push_back(std::make_pair(std::string(name), fs->FindIsDirectory(find))); }
		while ((name = fs->FindNext(find)));
		fs->FindClose(find);
	}
	return result;
}

int main(int argc, char **argv)
{
	assert(argc >= 3);
	const std::string gameID = "GAME", modID = "MOD";
	SourceAbiSearchPath paths[] = {
		{Span(modID), -1, 0}, {Span(gameID), -1, 0},
		{Span(gameID), -2, SOURCE_SEARCH_PACK | SOURCE_SEARCH_MAP | SOURCE_SEARCH_EXCLUDED},
		{Span(gameID), -2, SOURCE_SEARCH_PACK | SOURCE_SEARCH_MAP}
	};
	uint64_t plan = 0;
	assert(source_rust_bridge_search_plan_create(paths, 4, Span("GAME"), 1, 0, &plan) == SOURCE_ABI_OK);
	uint32_t ordinal = UINT32_MAX;
	assert(source_rust_bridge_search_plan_next(plan, &ordinal) == SOURCE_ABI_OK && ordinal == 1);
	assert(source_rust_bridge_search_plan_next(plan, &ordinal) == SOURCE_ABI_OK && ordinal == 3);
	assert(source_rust_bridge_search_plan_next(plan, &ordinal) == SOURCE_ABI_NOT_FOUND && ordinal == UINT32_MAX);
	assert(source_rust_bridge_search_state_reset(plan) == SOURCE_ABI_OK);
	assert(source_rust_bridge_search_plan_next(plan, &ordinal) == SOURCE_ABI_OK && ordinal == 1);
	assert(source_rust_bridge_search_state_destroy(plan) == SOURCE_ABI_OK);
	// Pure selection must work before activating any filesystem context.
	uint32_t selected = 99;
	assert(source_rust_bridge_read_path_matches("GAME", 4, "bSp", 3,
		true, true, true, &selected) == SOURCE_ABI_OK && selected == 1);
	assert(source_rust_bridge_read_path_matches("GAME", 4, "BSP", 3,
		true, false, false, &selected) == SOURCE_ABI_OK && selected == 0);
	assert(source_rust_bridge_read_path_matches("GAME", 4, "", 0,
		true, false, true, &selected) == SOURCE_ABI_OK && selected == 0);
	assert(source_rust_bridge_read_path_matches("GAME", 4, NULL, 0,
		false, false, true, &selected) == SOURCE_ABI_OK && selected == 1);
	CommandLine()->CreateCmdLine("pak_native_read -nomessagebox");
	void *module = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
	if (!module) { std::fprintf(stderr, "%s\n", dlerror()); return 1; }
	auto factory = reinterpret_cast<CreateInterfaceFn>(dlsym(module, "CreateInterface"));
	assert(factory);
	auto fs = static_cast<IFileSystem *>(factory(FILESYSTEM_INTERFACE_VERSION, NULL));
	assert(fs && fs->Connect(NoInterface) && fs->Init() == INIT_OK);
	SourceAbiContextConfig config = {sizeof(config), SOURCE_ABI_VERSION, NULL, NULL};
	SourceAbiHandle context = 0;
	assert(source_context_create(&config, &context) == SOURCE_ABI_OK);
	assert(source_rust_bridge_activate(context) == SOURCE_ABI_OK);
	size_t files = 0;
	for (int arg = 2; arg < argc; ++arg) {
		const std::string path = argv[arg];
		if (path.find("path-selection") != std::string::npos) {
			fs->AddSearchPath((path + "/loose").c_str(), "GAME", PATH_ADD_TO_HEAD);
			fs->AddSearchPath((path + "/zip").c_str(), "GAME", PATH_ADD_TO_HEAD);
			fs->AddSearchPath((path + "/literal").c_str(), "BSP", PATH_ADD_TO_HEAD);
			assert(!fs->FileExists("choice.txt", "BSP"));
			assert(fs->GetFileTime("choice.txt", "BSP") == 0);
			assert(FindPack(fs, "*").empty());
			fs->AddSearchPath((path + "/map.bsp").c_str(), "GAME", PATH_ADD_TO_TAIL);
			auto expect = [&](const char *pathID, char expected) {
				auto file = fs->Open("choice.txt", "rb", pathID);
				char byte = 0;
				assert(file && fs->Read(&byte, 1, file) == 1 && byte == expected);
				fs->Close(file);
			};
			expect("GAME", 'Z');
			expect("BSP", 'M');
			auto text = fs->Open("dir/lines.txt", "rt", "BSP");
			assert(text);
			char line[32];
			assert(fs->ReadLine(line, sizeof(line), text) && !std::strcmp(line, "first\n"));
			assert(fs->ReadLine(line, sizeof(line), text) && !std::strcmp(line, "second\n"));
			assert(fs->ReadLine(line, sizeof(line), text) && !std::strcmp(line, "last"));
			assert(!fs->ReadLine(line, sizeof(line), text));
			fs->Close(text);
			assert(fs->FileExists("map-only.txt", "BSP"));
			for (const char *name : {"loose-only.txt", "literal-only.txt", "zip-only.txt"}) {
				assert(!fs->FileExists(name, "BSP"));
				assert(fs->GetFileTime(name, "BSP") == 0); // Native iterator, not fast Rust reads.
			}
			assert(fs->GetFileTime("map-only.txt", "bSp") > 0);
			assert((FindPack(fs, "*") == Matches{{"choice.txt", false}, {"map-only.txt", false}, {"dir", true}}));
			fs->MarkPathIDByRequestOnly("GAME", true);
			assert(!fs->FileExists("map-only.txt"));
			assert(fs->GetFileTime("map-only.txt") == 0);
			assert(fs->GetFileTime("map-only.txt", "BSP") > 0);
			expect("GAME", 'Z');
			expect("BSP", 'M');
			fs->MarkPathIDByRequestOnly("GAME", false);
			assert(fs->FileExists("map-only.txt"));
			// GetSearchPath always uses the iterator (not fast Rust file reads).
			// A repeated physical directory under another ID shares a store ID.
			const std::string loose = path + "/loose/";
			fs->AddSearchPath((path + "/loose").c_str(), "ALIAS", PATH_ADD_TO_HEAD);
			auto listed = [&](const char *id, bool packs) {
				char output[4096];
				const int needed = fs->GetSearchPath(id, packs, output, sizeof(output));
				assert(needed == static_cast<int>(std::strlen(output)) + 1);
				assert(fs->GetSearchPath(id, packs, NULL, 0) == needed);
				return std::string(output);
			};
			for (int repeat = 0; repeat < 1000; ++repeat) {
				const std::string all = listed(NULL, false);
				const size_t at = all.find(loose);
				assert(at != std::string::npos && all.find(loose, at + 1) == std::string::npos);
				assert(listed("ALIAS", false) == loose);
				assert(listed("GAME", false).find(loose) != std::string::npos);
				assert(listed("BSP", false).empty());
				assert(listed("BSP", true) == path + "/map.bsp/");
			}
			fs->MarkPathIDByRequestOnly("ALIAS", true);
			assert(listed(NULL, false).find(loose) != std::string::npos);
			assert(listed("ALIAS", false) == loose);
			fs->MarkPathIDByRequestOnly("GAME", true);
			assert(listed(NULL, false).find(loose) == std::string::npos);
			fs->MarkPathIDByRequestOnly("ALIAS", false);
			fs->MarkPathIDByRequestOnly("GAME", false);
			fs->RemoveAllSearchPaths();
			assert(listed(NULL, false).empty()); // Empty/absolute pseudo-path stays native.
			// Exercise registry insertion, duplicate no-op, reordering, stable
			// removal and native FastRemove swap semantics with real resources.
			const std::string literal = path + "/literal/";
			fs->AddSearchPath((path + "/loose").c_str(), "GAME", PATH_ADD_TO_HEAD);
			fs->AddSearchPath((path + "/literal").c_str(), "MOD", PATH_ADD_TO_TAIL);
			assert(listed(NULL, false) == loose + ";" + literal);
			fs->AddSearchPath((path + "/loose").c_str(), "GAME", PATH_ADD_TO_TAIL);
			assert(listed(NULL, false) == loose + ";" + literal);
			fs->AddSearchPath((path + "/literal").c_str(), "GAME", PATH_ADD_TO_HEAD);
			assert(listed("GAME", false) == literal + ";" + loose);
			fs->AddSearchPath((path + "/loose").c_str(), "GAME", PATH_ADD_TO_HEAD);
			assert(listed("GAME", false) == loose + ";" + literal);
			fs->RemoveSearchPaths("GAME");
			assert(listed("GAME", false).empty() && listed(NULL, false) == literal);
			assert(fs->RemoveSearchPath((path + "/literal").c_str(), "MOD"));
			assert(listed(NULL, false).empty());
			std::puts("Native mount table: head/tail insertion, duplicate no-op, re-add movement, stable/fast removal and resource snapshots passed");
			std::puts("Native search plans: 1000 ordered alias/dedup/filter/size cycles, by-request refresh and empty snapshot passed");
			// Force the fallback find path, whose store visits also belong to Rust.
			// It must work without an activated context during early startup.
			source_rust_bridge_deactivate(context);
			fs->AddSearchPath((path + "/loose").c_str(), "GAME", PATH_ADD_TO_HEAD);
			fs->AddSearchPath((path + "/loose").c_str(), "ALIAS", PATH_ADD_TO_HEAD);
			for (int repeat = 0; repeat < 1000; ++repeat) {
				FileFindHandle_t find;
				std::set<std::string> names;
				const char *name = fs->FindFirstEx("*.txt", NULL, &find);
				assert(name);
				do { assert(names.insert(name).second); } while ((name = fs->FindNext(find)));
				fs->FindClose(find);
				assert((names == std::set<std::string>{"choice.txt", "loose-only.txt"}));
				assert(fs->FindFirstEx("*.txt", NULL, &find));
				fs->FindClose(find);
			}
			assert(source_rust_bridge_activate(context) == SOURCE_ABI_OK);
			fs->RemoveAllSearchPaths();
			std::puts("Native fallback finds: 1000 context-independent visit-set lifecycles and early closes passed");
			std::puts("Native selection: BSP excludes loose/literal/standalone ZIP mounts; GAME order, by-request flags, native timestamps and long-name filtering passed");
			continue;
		}
		if (path.find("shared-owner") != std::string::npos) {
			const std::string archive = path + "/a.bsp";
			auto expect = [&](char expected) {
				for (const char *pathID : {"GAME", "BSP"}) {
					auto file = fs->Open("transition.txt", "rb", pathID);
					char byte = 0;
					assert(file && fs->Read(&byte, 1, file) == 1 && byte == expected);
					fs->Close(file);
				}
			};
			fs->AddSearchPath(archive.c_str(), "GAME", PATH_ADD_TO_HEAD);
			expect('A');
			auto retained = fs->Open("transition.txt", "rb", "BSP");
			assert(retained);
			assert(std::rename(archive.c_str(), (path + "/original.bsp").c_str()) == 0);
			assert(std::rename((path + "/replacement.bsp").c_str(), archive.c_str()) == 0);
			// Unrelated mount synchronization must share the original descriptor,
			// not silently parse new bytes under the mounted archive's pathname.
			fs->AddSearchPath((path + "/extra").c_str(), "OTHER", PATH_ADD_TO_TAIL);
			expect('A');
			assert(fs->FileExists("only-a.txt", "GAME") && fs->FileExists("only-a.txt", "BSP"));
			assert(!fs->FileExists("only-b.txt", "GAME") && !fs->FileExists("only-b.txt", "BSP"));
			fs->RemoveAllSearchPaths();
			fs->AddSearchPath(archive.c_str(), "GAME", PATH_ADD_TO_HEAD);
			expect('B');
			char byte = 0;
			assert(fs->Read(&byte, 1, retained) == 1 && byte == 'A');
			fs->Close(retained);
			fs->RemoveAllSearchPaths();
			std::puts("Native shared owner: GAME/BSP retain the same archive through pathname replacement and mount resync; fresh remount sees new bytes");
			continue;
		}
		if (path.find("map-transitions") != std::string::npos) {
			for (auto order : {PATH_ADD_TO_HEAD, PATH_ADD_TO_TAIL}) {
				for (int cycle = 0; cycle < 100; ++cycle) {
					fs->AddSearchPath((path + "/a.bsp").c_str(), "GAME", order);
					fs->BeginMapAccess();
					fs->BeginMapAccess();
					// Repeated registration of the current map must be harmless.
					fs->AddSearchPath((path + "/a.bsp").c_str(), "GAME", order);
					auto retained = fs->Open("transition.txt", "rb", "BSP");
					assert(retained);
					assert(fs->FileExists("only-a.txt", "BSP"));
					assert(fs->FileExists("only-a.txt", "GAME"));
					fs->EndMapAccess();
					fs->EndMapAccess();
					fs->AddSearchPath((path + "/b.bsp").c_str(), "GAME", order);
					assert(!fs->FileExists("only-a.txt", "BSP"));
					assert(!fs->FileExists("only-a.txt", "GAME"));
					assert(fs->FileExists("only-b.txt", "BSP"));
					assert(fs->FileExists("only-b.txt", "GAME"));
					assert((FindPack(fs, "*.txt") == Matches{{"only-b.txt", false}, {"transition.txt", false}}));
					for (const char *pathID : {"GAME", "BSP"}) {
						auto current = fs->Open("transition.txt", "rb", pathID);
						char byte = 0;
						assert(current && fs->Read(&byte, 1, current) == 1 && byte == 'B');
						fs->Close(current);
					}
					fs->RemoveAllSearchPaths();
					char byte = 0;
					assert(fs->Read(&byte, 1, retained) == 1 && byte == 'A');
					fs->Close(retained);
					assert(!fs->FileExists("transition.txt", "BSP"));
					assert(!fs->FileExists("transition.txt", "GAME"));
				}
			}
			std::puts("Native map transitions: 200 replacement/remount cycles, nested map access, GAME/BSP precedence, wildcard refresh and retained old-map payloads passed");
			continue;
		}
		if (path.find("wildcards.bsp") != std::string::npos) {
			fs->AddSearchPath(path.c_str(), "GAME", PATH_ADD_TO_HEAD);
			assert((FindPack(fs, "*.*") == Matches{{"readme", false}, {"root.multi.txt", false}, {"dir", true}}));
			assert((FindPack(fs, "DIR/*.VMT") == Matches{{"a.vmt", false}, {"b.vmt", false}, {"multi.part.vmt", false}, {"\xc3\xa9.vmt", false}}));
			assert((FindPack(fs, "dir/*a*.?mt") == Matches{{"a.vmt", false}, {"multi.part.vmt", false}}));
			assert((FindPack(fs, "dir/s*") == Matches{{"same", false}, {"sub", true}}));
			assert((FindPack(fs, "dir/dotted.*") == Matches{{"dotted.dir", true}}));
			assert((FindPack(fs, "root.multi.*") == Matches{{"root.multi.txt", false}}));
			assert(FindPack(fs, "dir\\.\\sub\\..\\*.vmt") == FindPack(fs, "dir/*.vmt"));
			assert(FindPack(fs, "dir/missing*").empty());
			assert(FindPack(fs, "../*").empty());
			for (int i = 0; i < 1000; ++i) {
				assert((FindPack(fs, "dir/sub/*") == Matches{{"x.txt", false}, {"y.txt", false}}));
				FileFindHandle_t find;
				assert(fs->FindFirstEx("dir/*", "BSP", &find));
				fs->FindClose(find); // Close with directory/name lists still pending.
			}
			fs->RemoveAllSearchPaths();
			continue;
		}
		if (path.find("mount-series") != std::string::npos) {
			const bool numeric = path.find("numeric") != std::string::npos;
			for (auto order : {PATH_ADD_TO_HEAD, PATH_ADD_TO_TAIL}) {
				fs->AddSearchPath(path.c_str(), "GAME", order);
				auto file = fs->Open("priority.txt", "rb", "GAME");
				char byte = 0;
				assert(file && fs->Read(&byte, 1, file) == 1 &&
					byte == (numeric ? 'X' : order == PATH_ADD_TO_HEAD ? '2' : 'L'));
				fs->Close(file);
				assert(fs->FileExists("only0.txt", "GAME"));
				assert(fs->FileExists("only2.txt", "GAME"));
				assert(fs->FileExists("only4.txt", "GAME") == numeric);
				assert(!fs->FileExists("only12.txt", "GAME"));
				// Reuse the same native pack under a different path ID, then remove
				// its first mount. The Rust index/payload must remain live.
				fs->AddSearchPath(path.c_str(), "SECOND", PATH_ADD_TO_HEAD);
				fs->RemoveSearchPath(path.c_str(), "GAME");
				file = fs->Open("priority.txt", "rb", "SECOND");
				assert(file && fs->Read(&byte, 1, file) == 1 && byte == (numeric ? 'X' : '2'));
				fs->Close(file);
				fs->RemoveAllSearchPaths();
				if (!numeric) {
					fs->AddSearchPath((path + "/baseline").c_str(), "GAME", PATH_ADD_TO_HEAD);
					fs->AddSearchPath(path.c_str(), "GAME", order);
					file = fs->Open("priority.txt", "rb", "GAME");
					assert(file && fs->Read(&byte, 1, file) == 1 && byte == (order == PATH_ADD_TO_HEAD ? '2' : 'B'));
					fs->Close(file);
					fs->RemoveAllSearchPaths();
				}
			}
			continue;
		}
		if (path.find(".reject.bsp") != std::string::npos || path.find(".empty.bsp") != std::string::npos) {
			fs->AddSearchPath(path.c_str(), "GAME", PATH_ADD_TO_HEAD);
			assert(!fs->Open("a.txt", "rb", "BSP"));
			assert(!fs->Open((path + "/a.txt").c_str(), "rb"));
			fs->RemoveAllSearchPaths();
			continue;
		}
		if (path.size() >= 7 && path.substr(path.size() - 7) == ".badzip") {
			assert(fs->AddPackFile(path.c_str(), "GAME"));
			const std::string absolute = path + "/bad.txt";
			assert(!fs->Open(absolute.c_str(), "rb"));
			fs->RemoveAllSearchPaths();
			continue;
		}
		std::ifstream input(path, std::ios::binary);
		const std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
		const bool map = path.substr(path.size() - 4) == ".bsp";
		std::string zip = bytes;
		if (map) {
			assert(bytes.size() >= 1036);
			uint32_t offset, length;
			std::memcpy(&offset, bytes.data() + 8 + 40*16, 4);
			std::memcpy(&length, bytes.data() + 12 + 40*16, 4);
			assert(static_cast<uint64_t>(offset) + length <= bytes.size());
			zip = bytes.substr(offset, length);
		}
		SourceAbiHandle index = 0;
		uint32_t count = 0;
		assert(source_pak_index_create(Span(zip), &index, &count) == SOURCE_ABI_OK);
		if (map) fs->AddSearchPath(path.c_str(), "GAME", PATH_ADD_TO_HEAD);
		else assert(fs->AddPackFile(path.c_str(), "GAME"));
		std::set<std::string> expectedNames;
		FileHandle_t retained = NULL;
		std::vector<uint8_t> retainedBytes;
		for (uint32_t at = 0; at < count; ++at) {
			SourceAbiPakEntry entry = {};
			uint64_t written = 0;
			char name[1025] = {};
			assert(source_pak_index_entry(index, at, &entry, reinterpret_cast<uint8_t *>(name), sizeof(name)-1, &written) == SOURCE_ABI_OK);
			if (map) expectedNames.insert(std::string(name).substr(std::string(name).find_last_of('/') + 1));
			std::vector<uint8_t> expected(entry.length), actual(entry.length);
			SourceAbiMutSlice output = {expected.data(), expected.size()};
			assert(source_context_read_file(context, Span(name), Span("GAME"), output, &written) == SOURCE_ABI_OK);
			assert(written == entry.length);
			const std::string absolute = path + "/" + name;
			auto file = fs->Open(absolute.c_str(), "rb");
			assert(file && fs->Size(file) == entry.length);
			if (entry.length) {
				assert(fs->Read(actual.data(), actual.size(), file) == static_cast<int>(actual.size()));
				assert(actual == expected);
				const int tail = entry.length < 17 ? entry.length : 17;
				fs->Seek(file, -tail, FILESYSTEM_SEEK_TAIL);
				assert(fs->Tell(file) == entry.length - tail);
				assert(fs->Read(actual.data(), tail, file) == tail);
				assert(!std::memcmp(actual.data(), expected.data() + entry.length - tail, tail));
				fs->Seek(file, -1, FILESYSTEM_SEEK_HEAD);
				assert(fs->Tell(file) == 0);
				uint8_t probe[5] = {0xfe, 0xfe, 0xfe, 0xfe, 0xfe};
				const int shortRead = entry.length < 3 ? entry.length : 3;
				assert(fs->ReadEx(probe, 3, 8, file) == shortRead);
				assert(!std::memcmp(probe, expected.data(), shortRead));
				assert(probe[3] == 0xfe && probe[4] == 0xfe);
				fs->Seek(file, entry.length + 100, FILESYSTEM_SEEK_HEAD);
				assert(fs->Tell(file) == entry.length);
				fs->Seek(file, entry.length / 2, FILESYSTEM_SEEK_HEAD);
				assert(fs->Read(actual.data(), 1, file) == 1);
				assert(actual[0] == expected[entry.length / 2]);
				if (retained) fs->Close(retained);
				retained = fs->Open(absolute.c_str(), "rb");
				assert(retained);
				retainedBytes = expected;
			}
			fs->Close(file);
			if (map) {
				file = fs->Open(name, "rb", "BSP");
				assert(file && fs->Size(file) == entry.length);
				assert(fs->Read(actual.data(), actual.size(), file) == static_cast<int>(actual.size()));
				assert(actual == expected);
				fs->Close(file);
			}
			if (!std::strcmp(name, "lines.txt")) {
				file = fs->Open(absolute.c_str(), "rt");
				assert(file);
				char line[32];
				assert(fs->ReadLine(line, sizeof(line), file) && !std::strcmp(line, "first\n"));
				assert(fs->ReadLine(line, sizeof(line), file) && !std::strcmp(line, "second\n"));
				assert(fs->ReadLine(line, sizeof(line), file) && !std::strcmp(line, "last"));
				assert(!fs->ReadLine(line, sizeof(line), file));
				fs->Close(file);
			}
			++files;
		}
		if (map) {
			// BSP-only enumeration uses the public adapter over Rust's selected
			// map mounts, excluding every ordinary standalone archive.
			FileFindHandle_t find;
			std::set<std::string> names;
			const char *name = fs->FindFirstEx("materials/*.vmt", "BSP", &find);
			assert(name);
			do { assert(!fs->FindIsDirectory(find)); names.insert(name); } while ((name = fs->FindNext(find)));
			fs->FindClose(find);
			assert(names == expectedNames);
		}
		fs->RemoveAllSearchPaths();
		assert(source_pak_index_destroy(index) == SOURCE_ABI_OK);
		if (retained) {
			std::vector<uint8_t> actual(retainedBytes.size());
			assert(fs->Read(actual.data(), actual.size(), retained) == static_cast<int>(actual.size()));
			assert(actual == retainedBytes);
			fs->Close(retained);
		}
	}
	fs->Shutdown();
	fs->Disconnect();
	assert(dlclose(module) == 0);
	source_rust_bridge_deactivate(context);
	assert(source_context_destroy(context) == SOURCE_ABI_OK);
	std::printf("Native filesystem Rust-pack differential: %zu payload files checked; all supplied fixture assertions and clean unload passed\n", files);
}
