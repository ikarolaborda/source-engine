#include "rust/source_abi.h"
#include <cassert>
#include <cstdint>
#include <cstdio>
#include <utility>
#include <vector>
#include <thread>

using Step = std::pair<uint32_t, uint32_t>;
struct Scenario {
	uint32_t count;
	Step failure;
	bool nested;
	std::vector<Step> trace;
};

static int32_t Execute(void *data, uint32_t operation, uint32_t index)
{
	Scenario &s = *static_cast<Scenario *>(data);
	s.trace.emplace_back(operation, index);
	if (s.failure == Step(operation, index)) return -1;
	if (operation == SOURCE_APP_CREATE) return static_cast<int32_t>(s.count);
	if (operation == SOURCE_APP_MAIN) {
		if (s.nested) {
			Scenario child = {3, Step(SOURCE_APP_INIT, 1), false, {}};
			int32_t result = 123;
			assert(source_host_run_app_system_group(Execute, &child, &result) == SOURCE_ABI_OK);
			assert(result == -1 && child.trace.back().first == SOURCE_APP_DESTROY);
		}
		return 42;
	}
	return 0;
}

static int32_t WrongCallback(void *, uint32_t, uint32_t) { assert(false); return 0; }

struct Reentrant {
	uint64_t handle = 0;
	int creates = 0;
	int destroys = 0;
};

static int32_t Reenter(void *data, uint32_t operation, uint32_t)
{
	Reentrant &s = *static_cast<Reentrant *>(data);
	if (operation == SOURCE_APP_CREATE || operation == SOURCE_APP_DESTROY) {
		uint64_t nestedHandle = 999;
		int32_t result = 999;
		assert(source_host_app_group_startup(Reenter, data, &nestedHandle, &result) == SOURCE_ABI_INVALID_ARGUMENT);
		assert(nestedHandle == 0 && result == -1);
		assert(source_host_run_app_system_group(Reenter, data, &result) == SOURCE_ABI_INVALID_ARGUMENT);
		if (operation == SOURCE_APP_DESTROY) {
			++s.destroys;
			assert(source_host_app_group_shutdown(s.handle, Reenter, data) == SOURCE_ABI_INVALID_ARGUMENT);
		} else {
			++s.creates;
		}
		// A different identity is allowed to start and stop during a callback.
		Scenario child = {1, Step(UINT32_MAX,0), false, {}};
		assert(source_host_app_group_startup(Execute, &child, &nestedHandle, &result) == SOURCE_ABI_OK);
		assert(result == 0 && nestedHandle != 0);
		assert(source_host_app_group_shutdown(nestedHandle, Execute, &child) == SOURCE_ABI_OK);
	}
	return 0; // Empty group; Create count is zero.
}

int main()
{
	int32_t result = 123;
	assert(source_host_run_app_system_group(nullptr, nullptr, &result) == SOURCE_ABI_INVALID_ARGUMENT);
	assert(source_host_run_app_system_group(Execute, nullptr, nullptr) == SOURCE_ABI_INVALID_ARGUMENT);
	uint64_t invalidHandle = 0;
	assert(source_host_app_group_startup(nullptr, nullptr, &invalidHandle, &result) == SOURCE_ABI_INVALID_ARGUMENT);
	assert(source_host_app_group_startup(WrongCallback, nullptr, nullptr, &result) == SOURCE_ABI_INVALID_ARGUMENT);
	assert(source_host_app_group_startup(WrongCallback, nullptr, &invalidHandle, nullptr) == SOURCE_ABI_INVALID_ARGUMENT);
	assert(source_host_app_group_shutdown(0, WrongCallback, nullptr) == SOURCE_ABI_INVALID_HANDLE);
	assert(source_host_app_group_shutdown(0, nullptr, nullptr) == SOURCE_ABI_INVALID_ARGUMENT);
	unsigned cases = 0;
	unsigned splitCases = 0;
	for (uint32_t count = 0; count < 32; ++count) {
		std::vector<Step> failures = {Step(UINT32_MAX, 0), Step(SOURCE_APP_CREATE, 0), Step(SOURCE_APP_PREINIT, 0)};
		for (uint32_t i = 0; i < count; ++i) {
			failures.emplace_back(SOURCE_APP_CONNECT, i);
			failures.emplace_back(SOURCE_APP_INIT, i);
		}
		for (const Step &failure : failures) {
			Scenario s = {count, failure, true, {}};
			assert(source_host_run_app_system_group(Execute, &s, &result) == SOURCE_ABI_OK);
			const bool success = failure.first == UINT32_MAX;
			assert(result == (success ? 42 : -1));
			std::vector<Step> expected;
			bool started = true;
			auto step = [&](uint32_t op, uint32_t index) {
				expected.emplace_back(op, index);
				return failure != Step(op, index);
			};
			uint32_t connected = 0, initialized = 0;
			bool preinit = false;
			started = step(SOURCE_APP_CREATE, 0);
			while (started && connected < count) {
				started = step(SOURCE_APP_CONNECT, connected);
				if (started) ++connected;
			}
			if (started) preinit = started = step(SOURCE_APP_PREINIT, 0);
			while (started && initialized < count) {
				started = step(SOURCE_APP_INIT, initialized);
				if (started) ++initialized;
			}
			if (started) step(SOURCE_APP_MAIN, 0);
			while (initialized) step(SOURCE_APP_SHUTDOWN, --initialized);
			if (preinit) step(SOURCE_APP_POSTSHUTDOWN, 0);
			while (connected) step(SOURCE_APP_DISCONNECT, --connected);
			step(SOURCE_APP_REMOVE_SYSTEMS, 0);
			step(SOURCE_APP_UNLOAD_MODULES, 0);
			step(SOURCE_APP_DESTROY, 0);
			assert(s.trace == expected);
			// Both public lifecycles must have identical startup/rollback traces,
			// except that the split form never calls Main itself.
			Scenario split = {count, failure, false, {}};
			uint64_t handle = 999;
			assert(source_host_app_group_startup(Execute, &split, &handle, &result) == SOURCE_ABI_OK);
			assert(result == (success ? 0 : -1));
			if (success) {
				assert(handle != 0 && split.trace.back().first == (count ? SOURCE_APP_INIT : SOURCE_APP_PREINIT));
				const size_t before = split.trace.size();
				uint64_t duplicate = 999;
				assert(source_host_app_group_startup(Execute, &split, &duplicate, &result) == SOURCE_ABI_INVALID_ARGUMENT);
				assert(duplicate == 0 && result == -1 && split.trace.size() == before);
				assert(source_host_run_app_system_group(Execute, &split, &result) == SOURCE_ABI_INVALID_ARGUMENT);
				assert(source_host_app_group_shutdown(handle, WrongCallback, &split) == SOURCE_ABI_INVALID_ARGUMENT);
				assert(source_host_app_group_shutdown(handle, Execute, &s) == SOURCE_ABI_INVALID_ARGUMENT);
				std::thread wrongThread([&] {
					assert(source_host_app_group_shutdown(handle, Execute, &split) == SOURCE_ABI_INVALID_ARGUMENT);
				});
				wrongThread.join();
				assert(split.trace.size() == before);
				assert(source_host_app_group_shutdown(handle, Execute, &split) == SOURCE_ABI_OK);
				assert(source_host_app_group_shutdown(handle, Execute, &split) == SOURCE_ABI_INVALID_HANDLE);
			} else {
				assert(handle == 0);
			}
			std::vector<Step> noMain;
			for (const Step &step : expected) if (step.first != SOURCE_APP_MAIN) noMain.push_back(step);
			assert(split.trace == noMain);
			++splitCases;
			++cases;
		}
	}
	std::printf("App-system ABI lifecycle: %u startup/rollback cases and nested groups passed\n", cases);
	uint64_t previousHandle = 0;
	for (int cycle = 0; cycle < 1000; ++cycle) {
		Reentrant s;
		assert(source_host_app_group_startup(Reenter, &s, &s.handle, &result) == SOURCE_ABI_OK);
		assert(result == 0 && s.handle != 0);
		assert(s.handle != previousHandle);
		assert(source_host_app_group_shutdown(previousHandle, Reenter, &s) == SOURCE_ABI_INVALID_HANDLE);
		assert(source_host_app_group_shutdown(s.handle, Reenter, &s) == SOURCE_ABI_OK);
		assert(s.creates == 1 && s.destroys == 1);
		assert(source_host_app_group_shutdown(s.handle, Reenter, &s) == SOURCE_ABI_INVALID_HANDLE);
		previousHandle = s.handle;
	}
	std::printf("Split app-system ABI: %u startup/rollback cases, ownership guards, 1000 reentrant cycles passed\n", splitCases);
}
