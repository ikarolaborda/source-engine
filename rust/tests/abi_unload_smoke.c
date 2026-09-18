#include "rust/source_abi.h"

#include <dlfcn.h>
#include <stdio.h>

typedef uint32_t (*version_fn)(void);
typedef SourceAbiStatus (*create_fn)(const SourceAbiContextConfig *, SourceAbiHandle *);
typedef SourceAbiStatus (*destroy_fn)(SourceAbiHandle);

int main(int argc, char **argv)
{
	if (argc != 2)
		return 2;
	for (int iteration = 0; iteration < 128; ++iteration)
	{
		void *library = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
		if (!library)
		{
			fprintf(stderr, "%s\n", dlerror());
			return 3;
		}
		version_fn version = (version_fn)dlsym(library, "source_abi_version");
		create_fn create = (create_fn)dlsym(library, "source_context_create");
		destroy_fn destroy = (destroy_fn)dlsym(library, "source_context_destroy");
		if (!version || !create || !destroy || version() != SOURCE_ABI_VERSION)
			return 4;

		SourceAbiContextConfig config = {
			sizeof(SourceAbiContextConfig), SOURCE_ABI_VERSION, NULL, NULL
		};
		SourceAbiHandle handle = 0;
		if (create(&config, &handle) != SOURCE_ABI_OK || destroy(handle) != SOURCE_ABI_OK)
			return 5;
		if (dlclose(library) != 0)
			return 6;
	}
	puts("C ABI unload smoke: 128 load/unload cycles passed");
	return 0;
}

