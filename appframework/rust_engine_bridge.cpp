#include "rust_engine_bridge.h"
#include "rust/source_d3d9.h"

#include <atomic>
#include <cstring>

namespace
{
std::atomic<SourceAbiHandle> g_ActiveRustEngineHandle( 0 );

// The Metal presenter attached to the game window, published here by the
// window manager so the engine's frame loop can reach it without the two
// modules having to know about each other. Zero until a window is created
// with -metal, and zero again once it is destroyed, which is what the
// frame loop tests to decide whether it has anywhere to draw.
std::atomic<SourceAbiHandle> g_MetalPresenterHandle( 0 );
}

extern "C" void source_rust_bridge_set_presenter( SourceAbiHandle presenter )
{
	g_MetalPresenterHandle.store( presenter, std::memory_order_release );
}

extern "C" SourceAbiHandle source_rust_bridge_presenter()
{
	return g_MetalPresenterHandle.load( std::memory_order_acquire );
}

extern "C" SourceAbiStatus source_rust_bridge_scene_load( const char *map,
	uint64_t mapLength, SourceAbiWorldDraw *drawn )
{
	const SourceAbiHandle context = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( context == 0 || presenter == 0 || map == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;

	SourceAbiSlice mapSlice;
	mapSlice.data = reinterpret_cast<const uint8_t *>( map );
	mapSlice.length = mapLength;
	return source_render_world_load( presenter, context, mapSlice, drawn );
}

extern "C" SourceAbiStatus source_rust_bridge_scene_present( const float *position,
	const float *angles, SourceAbiWorldDraw *drawn )
{
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( presenter == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_render_world_present( presenter, position, angles, drawn );
}

extern "C" SourceAbiStatus source_rust_bridge_activate( SourceAbiHandle handle )
{
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;

	uint32_t phase = 0;
	const SourceAbiStatus status = source_context_host_phase( handle, &phase );
	if ( status == SOURCE_ABI_OK )
		g_ActiveRustEngineHandle.store( handle, std::memory_order_release );
	return status;
}

extern "C" void source_rust_bridge_deactivate( SourceAbiHandle handle )
{
	SourceAbiHandle expected = handle;
	g_ActiveRustEngineHandle.compare_exchange_strong( expected, 0,
		std::memory_order_acq_rel );
}

extern "C" SourceAbiStatus source_rust_bridge_world_load( const char *virtualPath,
	uint64_t virtualPathLength, SourceAbiWorldInfo *info )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || virtualPath == 0 || info == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;

	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	static const char pathId[] = "GAME";
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = sizeof( pathId ) - 1;
	return source_context_world_load( handle, virtualPathSlice, pathIdSlice, info );
}

extern "C" SourceAbiStatus source_rust_bridge_world_clear()
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_world_clear( handle );
}

extern "C" SourceAbiStatus source_rust_bridge_read_file( const char *virtualPath,
	uint64_t virtualPathLength, const char *pathId, uint64_t pathIdLength,
	void *output, uint64_t outputLength, uint64_t *written )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || written == 0 ||
		( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) ||
		( outputLength != 0 && output == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}

	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	return source_context_read_file( handle, virtualPathSlice, pathIdSlice,
		outputSlice, written );
}

extern "C" SourceAbiStatus source_rust_bridge_file_size( const char *virtualPath,
	uint64_t virtualPathLength, const char *pathId, uint64_t pathIdLength,
	uint64_t *size )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || size == 0 ||
		( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}

	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_file_size( handle, virtualPathSlice, pathIdSlice, size );
}

extern "C" SourceAbiStatus source_rust_bridge_resolve_read_path(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, void *output, uint64_t outputLength, uint64_t *written,
	uint32_t *kind )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || written == 0 || kind == 0 ||
		( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) ||
		( outputLength != 0 && output == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	return source_context_resolve_read_path( handle, virtualPathSlice, pathIdSlice,
		outputSlice, written, kind );
}

extern "C" SourceAbiStatus source_rust_bridge_path_is_directory( const char *virtualPath,
	uint64_t virtualPathLength, const char *pathId, uint64_t pathIdLength,
	uint32_t *isDirectory )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || isDirectory == 0 ||
		( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}

	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_path_is_directory( handle, virtualPathSlice, pathIdSlice,
		isDirectory );
}

extern "C" SourceAbiStatus source_rust_bridge_find_first( const char *wildcard,
	uint64_t wildcardLength, const char *pathId, uint64_t pathIdLength,
	void *output, uint64_t outputLength, uint64_t *written,
	uint32_t *isDirectory, uint64_t *find )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || written == 0 || isDirectory == 0 || find == 0 ||
		( wildcardLength != 0 && wildcard == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) ||
		( outputLength != 0 && output == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice wildcardSlice;
	wildcardSlice.data = reinterpret_cast<const uint8_t *>( wildcard );
	wildcardSlice.length = wildcardLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	if ( outputLength == 0 || outputLength > UINT32_MAX ) return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_find_first_bounded( handle, wildcardSlice, pathIdSlice,
		static_cast<uint32_t>( outputLength ), outputSlice, written, isDirectory, find );
}

extern "C" SourceAbiStatus source_rust_bridge_find_next( uint64_t find,
	void *output, uint64_t outputLength, uint64_t *written, uint32_t *isDirectory )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || find == 0 || written == 0 || isDirectory == 0 ||
		( outputLength != 0 && output == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	return source_context_find_next( handle, find, outputSlice, written, isDirectory );
}

extern "C" SourceAbiStatus source_rust_bridge_find_close( uint64_t find )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return handle == 0 || find == 0 ? SOURCE_ABI_INVALID_ARGUMENT :
		source_context_find_close( handle, find );
}

extern "C" SourceAbiStatus source_rust_bridge_read_paths_clear()
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_read_paths_clear( handle );
}

extern "C" SourceAbiStatus source_rust_bridge_read_path_add_directory_flags(
	const char *root, uint64_t rootLength, const char *pathId,
	uint64_t pathIdLength, bool atHead, bool byRequestOnly, bool allowSymlinkEscape )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( rootLength != 0 && root == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice rootSlice;
	rootSlice.data = reinterpret_cast<const uint8_t *>( root );
	rootSlice.length = rootLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_read_path_add_directory_flags( handle, rootSlice,
		pathIdSlice, atHead ? 1 : 0, byRequestOnly ? 1 : 0,
		allowSymlinkEscape ? 1 : 0 );
}

extern "C" SourceAbiStatus source_rust_bridge_read_path_add_vpk_flags(
	const char *directoryPath, uint64_t directoryPathLength, const char *pathId,
	uint64_t pathIdLength, bool atHead, bool byRequestOnly )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( directoryPathLength != 0 && directoryPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice directoryPathSlice;
	directoryPathSlice.data = reinterpret_cast<const uint8_t *>( directoryPath );
	directoryPathSlice.length = directoryPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_read_path_add_vpk_flags( handle, directoryPathSlice,
		pathIdSlice, atHead ? 1 : 0, byRequestOnly ? 1 : 0 );
}

extern "C" SourceAbiStatus source_rust_bridge_write_paths_clear()
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_write_paths_clear( handle );
}

extern "C" SourceAbiStatus source_rust_bridge_run_app_system_group(
	SourceAbiAppSystemFn callback, void *userData, int32_t *result )
{
	return source_host_run_app_system_group( callback, userData, result );
}

extern "C" SourceAbiStatus source_rust_bridge_pak_index_destroy( uint64_t handle )
{
	return source_pak_index_destroy( handle );
}
extern "C" SourceAbiStatus source_rust_bridge_mount_table_create(SourceAbiMountDropFn dropFn, SourceAbiMountCloneFn cloneFn, uint64_t *handle)
{ return source_mount_table_create(dropFn, cloneFn, handle); }
extern "C" SourceAbiStatus source_rust_bridge_mount_table_insert(uint64_t handle, uint32_t index, void *resource)
{ return source_mount_table_insert(handle, index, resource); }
extern "C" SourceAbiStatus source_rust_bridge_mount_table_count(uint64_t handle, uint32_t *count)
{ return source_mount_table_count(handle, count); }
extern "C" SourceAbiStatus source_rust_bridge_mount_table_get(uint64_t handle, uint32_t index, void **resource)
{ return source_mount_table_get(handle, index, resource); }
extern "C" SourceAbiStatus source_rust_bridge_mount_table_remove(uint64_t handle, uint32_t index, uint8_t fast)
{ return source_mount_table_remove(handle, index, fast); }
extern "C" SourceAbiStatus source_rust_bridge_mount_table_clear(uint64_t handle)
{ return source_mount_table_clear(handle); }
extern "C" SourceAbiStatus source_rust_bridge_mount_table_snapshot(uint64_t handle, uint64_t *copy)
{ return source_mount_table_snapshot(handle, copy); }
extern "C" SourceAbiStatus source_rust_bridge_mount_table_destroy(uint64_t handle)
{ return source_mount_table_destroy(handle); }
extern "C" SourceAbiStatus source_rust_bridge_mount_store_id_next(int32_t *id)
{ return source_mount_store_id_next(id); }

extern "C" SourceAbiStatus source_rust_bridge_read_path_matches(
	const char *stored, uint64_t storedLength, const char *requested, uint64_t requestedLength,
	bool hasRequested, bool byRequestOnly, bool isMapPack, uint32_t *matches )
{
	SourceAbiSlice storedSlice = { reinterpret_cast<const uint8_t *>( stored ), storedLength };
	SourceAbiSlice requestedSlice = { reinterpret_cast<const uint8_t *>( requested ), requestedLength };
	return source_read_path_matches( storedSlice, requestedSlice, hasRequested ? 1 : 0,
		byRequestOnly ? 1 : 0, isMapPack ? 1 : 0, matches );
}

extern "C" SourceAbiStatus source_rust_bridge_search_plan_create(
	const SourceAbiSearchPath *paths, uint32_t count, SourceAbiSlice requested,
	uint8_t hasRequested, uint32_t filter, uint64_t *handle)
{
	return source_search_plan_create(paths, count, requested, hasRequested, filter, handle);
}
extern "C" SourceAbiStatus source_rust_bridge_search_plan_next(uint64_t handle, uint32_t *index)
{ return source_search_plan_next(handle, index); }
extern "C" SourceAbiStatus source_rust_bridge_search_visits_create(uint64_t *handle)
{ return source_search_visits_create(handle); }
extern "C" SourceAbiStatus source_rust_bridge_search_visits_mark(uint64_t handle, int32_t store, uint32_t *seen)
{ return source_search_visits_mark(handle, store, seen); }
extern "C" SourceAbiStatus source_rust_bridge_search_state_reset(uint64_t handle)
{ return source_search_state_reset(handle); }
extern "C" SourceAbiStatus source_rust_bridge_search_state_destroy(uint64_t handle)
{ return source_search_state_destroy(handle); }
extern "C" SourceAbiStatus source_rust_bridge_read_path_add_pak_index(
	uint64_t index, const char *pathId, uint64_t pathIdLength, bool atHead, bool byRequestOnly )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	SourceAbiSlice pathIdSlice = { reinterpret_cast<const uint8_t *>( pathId ), pathIdLength };
	return source_context_read_path_add_pak_index( handle, index, pathIdSlice,
		atHead ? 1 : 0, byRequestOnly ? 1 : 0 );
}
extern "C" SourceAbiStatus source_rust_bridge_find_pack_candidates(
	SourceAbiSlice root, uint32_t naming, SourceAbiSlice language, SourceAbiMutSlice output,
	uint64_t *written, uint32_t *isDirectory, uint64_t *find )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return source_context_find_pack_candidates( handle, root, naming, language, output, written, isDirectory, find );
}
extern "C" SourceAbiStatus source_rust_bridge_find_first_pak(
	uint64_t index, SourceAbiSlice pattern, SourceAbiMutSlice output,
	uint64_t *written, uint32_t *isDirectory, uint64_t *find )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return source_context_find_first_pak( handle, index, pattern, output, written, isDirectory, find );
}
extern "C" SourceAbiStatus source_rust_bridge_file_open_pak(
	uint64_t index, uint32_t entry, uint64_t *file, uint64_t *size, uint64_t *absoluteOffset )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return source_context_file_open_pak( handle, index, entry, file, size, absoluteOffset );
}
extern "C" SourceAbiStatus source_rust_bridge_pak_index_open_archive(
	SourceAbiSlice path, uint32_t kind, uint64_t *handle, SourceAbiPakArchiveInfo *info )
{
	return source_pak_index_open_archive( path, kind, handle, info );
}
extern "C" SourceAbiStatus source_rust_bridge_pak_index_find(
	uint64_t handle, SourceAbiSlice path, SourceAbiPakEntry *entry )
{
	return source_pak_index_find( handle, path, entry );
}
extern "C" SourceAbiStatus source_rust_bridge_pak_index_entry(
	uint64_t handle, uint32_t index, SourceAbiPakEntry *entry,
	uint8_t *name, uint64_t capacity, uint64_t *written )
{
	return source_pak_index_entry( handle, index, entry, name, capacity, written );
}

extern "C" SourceAbiStatus source_rust_bridge_app_group_startup(
	SourceAbiAppSystemFn callback, void *userData, uint64_t *handle, int32_t *result )
{
	return source_host_app_group_startup( callback, userData, handle, result );
}

extern "C" SourceAbiStatus source_rust_bridge_app_group_shutdown(
	uint64_t handle, SourceAbiAppSystemFn callback, void *userData )
{
	return source_host_app_group_shutdown( handle, callback, userData );
}

extern "C" SourceAbiStatus source_rust_bridge_write_path_add( const char *root,
	uint64_t rootLength, const char *pathId, uint64_t pathIdLength, bool atHead )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( rootLength != 0 && root == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}

	SourceAbiSlice rootSlice;
	rootSlice.data = reinterpret_cast<const uint8_t *>( root );
	rootSlice.length = rootLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_write_path_add( handle, rootSlice, pathIdSlice,
		atHead ? 1 : 0 );
}

extern "C" SourceAbiStatus source_rust_bridge_write_path_add_flags( const char *root,
	uint64_t rootLength, const char *pathId, uint64_t pathIdLength, bool atHead,
	bool byRequestOnly )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( rootLength != 0 && root == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}

	SourceAbiSlice rootSlice;
	rootSlice.data = reinterpret_cast<const uint8_t *>( root );
	rootSlice.length = rootLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_write_path_add_flags( handle, rootSlice, pathIdSlice,
		atHead ? 1 : 0, byRequestOnly ? 1 : 0 );
}

extern "C" SourceAbiStatus source_rust_bridge_resolve_write_path(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, void *output, uint64_t outputLength, uint64_t *written )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || written == 0 ||
		( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) ||
		( outputLength != 0 && output == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}

	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	return source_context_resolve_write_path( handle, virtualPathSlice, pathIdSlice,
		outputSlice, written );
}

extern "C" SourceAbiStatus source_rust_bridge_create_write_directory(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_create_write_directory( handle, virtualPathSlice, pathIdSlice );
}

extern "C" SourceAbiStatus source_rust_bridge_remove_write_file(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_remove_write_file( handle, virtualPathSlice, pathIdSlice );
}

extern "C" SourceAbiStatus source_rust_bridge_rename_write_file(
	const char *oldVirtualPath, uint64_t oldVirtualPathLength, const char *oldPathId,
	uint64_t oldPathIdLength, const char *newVirtualPath, uint64_t newVirtualPathLength,
	const char *newPathId, uint64_t newPathIdLength )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( oldVirtualPathLength != 0 && oldVirtualPath == 0 ) ||
		( oldPathIdLength != 0 && oldPathId == 0 ) ||
		( newVirtualPathLength != 0 && newVirtualPath == 0 ) ||
		( newPathIdLength != 0 && newPathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice oldVirtualPathSlice;
	oldVirtualPathSlice.data = reinterpret_cast<const uint8_t *>( oldVirtualPath );
	oldVirtualPathSlice.length = oldVirtualPathLength;
	SourceAbiSlice oldPathIdSlice;
	oldPathIdSlice.data = reinterpret_cast<const uint8_t *>( oldPathId );
	oldPathIdSlice.length = oldPathIdLength;
	SourceAbiSlice newVirtualPathSlice;
	newVirtualPathSlice.data = reinterpret_cast<const uint8_t *>( newVirtualPath );
	newVirtualPathSlice.length = newVirtualPathLength;
	SourceAbiSlice newPathIdSlice;
	newPathIdSlice.data = reinterpret_cast<const uint8_t *>( newPathId );
	newPathIdSlice.length = newPathIdLength;
	return source_context_rename_write_file( handle, oldVirtualPathSlice,
		oldPathIdSlice, newVirtualPathSlice, newPathIdSlice );
}

extern "C" SourceAbiStatus source_rust_bridge_is_write_file_writable(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, uint32_t *writable )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || writable == 0 ||
		( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_is_write_file_writable( handle, virtualPathSlice,
		pathIdSlice, writable );
}

extern "C" SourceAbiStatus source_rust_bridge_set_write_file_writable(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, bool writable )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_set_write_file_writable( handle, virtualPathSlice,
		pathIdSlice, writable ? 1 : 0 );
}

extern "C" SourceAbiStatus source_rust_bridge_file_open_read(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, uint64_t *file, uint64_t *size )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || file == 0 || size == 0 ||
		( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	return source_context_file_open_read( handle, virtualPathSlice, pathIdSlice,
		file, size );
}

extern "C" SourceAbiStatus source_rust_bridge_file_open_write(
	const char *virtualPath, uint64_t virtualPathLength, const char *pathId,
	uint64_t pathIdLength, const char *mode, uint64_t modeLength,
	uint64_t *file, uint64_t *size )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || file == 0 || size == 0 ||
		( virtualPathLength != 0 && virtualPath == 0 ) ||
		( pathIdLength != 0 && pathId == 0 ) || ( modeLength != 0 && mode == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = pathIdLength;
	SourceAbiSlice modeSlice;
	modeSlice.data = reinterpret_cast<const uint8_t *>( mode );
	modeSlice.length = modeLength;
	return source_context_file_open_write( handle, virtualPathSlice, pathIdSlice,
		modeSlice, file, size );
}

extern "C" SourceAbiStatus source_rust_bridge_file_close( uint64_t file )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return handle == 0 ? SOURCE_ABI_INVALID_ARGUMENT : source_context_file_close( handle, file );
}

extern "C" SourceAbiStatus source_rust_bridge_file_read( uint64_t file,
	void *output, uint64_t outputLength, uint64_t *read )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || read == 0 || ( outputLength != 0 && output == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	return source_context_file_read( handle, file, outputSlice, read );
}

extern "C" SourceAbiStatus source_rust_bridge_file_write( uint64_t file,
	const void *input, uint64_t inputLength, uint64_t *written )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || written == 0 || ( inputLength != 0 && input == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice inputSlice;
	inputSlice.data = static_cast<const uint8_t *>( input );
	inputSlice.length = inputLength;
	return source_context_file_write( handle, file, inputSlice, written );
}

extern "C" SourceAbiStatus source_rust_bridge_file_flush( uint64_t file )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return handle == 0 ? SOURCE_ABI_INVALID_ARGUMENT : source_context_file_flush( handle, file );
}

extern "C" SourceAbiStatus source_rust_bridge_file_seek( uint64_t file, int64_t offset,
	uint32_t origin, uint64_t *position )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return handle == 0 ? SOURCE_ABI_INVALID_ARGUMENT :
		source_context_file_seek( handle, file, offset, origin, position );
}

extern "C" SourceAbiStatus source_rust_bridge_file_tell( uint64_t file, uint64_t *position )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return handle == 0 ? SOURCE_ABI_INVALID_ARGUMENT :
		source_context_file_tell( handle, file, position );
}

extern "C" SourceAbiStatus source_rust_bridge_open_file_size( uint64_t file, uint64_t *size )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return handle == 0 ? SOURCE_ABI_INVALID_ARGUMENT :
		source_context_open_file_size( handle, file, size );
}

extern "C" SourceAbiStatus source_rust_bridge_file_is_open( uint64_t file, uint32_t *open )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	return handle == 0 ? SOURCE_ABI_INVALID_ARGUMENT :
		source_context_file_is_open( handle, file, open );
}

extern "C" SourceAbiStatus source_rust_bridge_net_channel_create(
	int32_t outgoingSequence, int32_t incomingSequence, int32_t outgoingAck,
	uint64_t *channelId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || channelId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_net_channel_create( handle, outgoingSequence,
		incomingSequence, outgoingAck, channelId );
}

extern "C" SourceAbiStatus source_rust_bridge_net_channel_remove( uint64_t channelId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || channelId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_net_channel_remove( handle, channelId );
}

extern "C" SourceAbiStatus source_rust_bridge_net_channel_reset( uint64_t channelId,
	int32_t outgoingSequence, int32_t incomingSequence, int32_t outgoingAck )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || channelId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_net_channel_reset( handle, channelId, outgoingSequence,
		incomingSequence, outgoingAck );
}

extern "C" SourceAbiStatus source_rust_bridge_net_channel_preview_incoming(
	uint64_t channelId, int32_t sequence, int32_t outgoingAck, uint32_t choked,
	int32_t maxDrop, SourceAbiNetPacketDecision *decision )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || channelId == 0 || decision == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_net_channel_preview_incoming( handle, channelId, sequence,
		outgoingAck, choked, maxDrop, decision );
}

extern "C" SourceAbiStatus source_rust_bridge_net_channel_commit_incoming(
	uint64_t channelId, int32_t sequence, int32_t outgoingAck, uint32_t choked,
	int32_t maxDrop, SourceAbiNetPacketDecision *decision )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || channelId == 0 || decision == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_net_channel_commit_incoming( handle, channelId, sequence,
		outgoingAck, choked, maxDrop, decision );
}

extern "C" SourceAbiStatus source_rust_bridge_net_channel_advance_outgoing(
	uint64_t channelId, SourceAbiNetSequenceAdvance *advance )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || channelId == 0 || advance == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_net_channel_advance_outgoing( handle, channelId, advance );
}

extern "C" SourceAbiStatus source_rust_bridge_net_packet_checksum(
	const void *payload, uint64_t payloadLength, uint16_t *checksum )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || checksum == 0 || ( payloadLength != 0 && payload == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice payloadSlice;
	payloadSlice.data = reinterpret_cast<const uint8_t *>( payload );
	payloadSlice.length = payloadLength;
	return source_context_net_packet_checksum( handle, payloadSlice, checksum );
}

extern "C" SourceAbiStatus source_rust_bridge_net_packet_encode_header(
	int32_t sequence, int32_t outgoingAck, uint32_t reliableState,
	bool hasChoked, uint32_t choked, bool hasChallenge, uint32_t challenge,
	bool checksumRequired, SourceAbiNetEncodedPacketHeader *header )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || header == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_net_packet_encode_header( handle, sequence, outgoingAck,
		reliableState, hasChoked ? 1u : 0u, choked, hasChallenge ? 1u : 0u,
		challenge, checksumRequired ? 1u : 0u, header );
}

extern "C" SourceAbiStatus source_rust_bridge_net_packet_finalize_header(
	void *packet, uint64_t packetLength, uint32_t flags, bool checksumRequired,
	uint16_t *checksum )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || checksum == 0 || ( packetLength != 0 && packet == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiMutSlice packetSlice;
	packetSlice.data = reinterpret_cast<uint8_t *>( packet );
	packetSlice.length = packetLength;
	return source_context_net_packet_finalize_header( handle, packetSlice, flags,
		checksumRequired ? 1u : 0u, checksum );
}

extern "C" SourceAbiStatus source_rust_bridge_net_packet_header(
	const void *packet, uint64_t packetLength, bool checksumRequired,
	bool expectsChallenge, uint32_t expectedChallenge, SourceAbiNetPacketHeader *header )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || header == 0 || ( packetLength != 0 && packet == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice packetSlice;
	packetSlice.data = reinterpret_cast<const uint8_t *>( packet );
	packetSlice.length = packetLength;
	return source_context_net_packet_header( handle, packetSlice,
		checksumRequired ? 1u : 0u, expectsChallenge ? 1u : 0u,
		expectedChallenge, header );
}

// The codec is stateless, so unlike the context-bearing calls above these do
// not need an engine handle and stay usable before one exists.
extern "C" SourceAbiStatus source_rust_bridge_lzss_compress( const void *input,
	uint64_t inputLength, uint32_t window, void *output, uint64_t capacity,
	uint64_t *length )
{
	if ( length == 0 || ( inputLength != 0 && input == 0 ) ||
		( capacity != 0 && output == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice inputSlice;
	inputSlice.data = reinterpret_cast<const uint8_t *>( input );
	inputSlice.length = inputLength;
	return source_compress_lzss_compress( inputSlice, window,
		reinterpret_cast<uint8_t *>( output ), capacity, length );
}

extern "C" SourceAbiStatus source_rust_bridge_lzss_decompress( const void *input,
	uint64_t inputLength, void *output, uint64_t capacity, uint64_t *length )
{
	if ( length == 0 || ( inputLength != 0 && input == 0 ) ||
		( capacity != 0 && output == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice inputSlice;
	inputSlice.data = reinterpret_cast<const uint8_t *>( input );
	inputSlice.length = inputLength;
	return source_compress_lzss_decompress( inputSlice,
		reinterpret_cast<uint8_t *>( output ), capacity, length );
}

extern "C" SourceAbiStatus source_rust_bridge_lzss_actual_size( const void *input,
	uint64_t inputLength, uint64_t *actualSize )
{
	if ( actualSize == 0 || ( inputLength != 0 && input == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice inputSlice;
	inputSlice.data = reinterpret_cast<const uint8_t *>( input );
	inputSlice.length = inputLength;
	return source_compress_lzss_actual_size( inputSlice, actualSize );
}

extern "C" SourceAbiStatus source_rust_bridge_snappy_max_size( uint64_t inputLength,
	uint64_t *size )
{
	return source_compress_snappy_max_size( inputLength, size );
}

extern "C" SourceAbiStatus source_rust_bridge_snappy_compress( const void *input,
	uint64_t inputLength, void *output, uint64_t capacity, uint64_t *length )
{
	SourceAbiSlice bytes = { reinterpret_cast<const uint8_t *>( input ), inputLength };
	return source_compress_snappy_compress( bytes, reinterpret_cast<uint8_t *>( output ),
		capacity, length );
}

extern "C" SourceAbiStatus source_rust_bridge_buffer_actual_size( const void *input,
	uint64_t inputLength, uint64_t *size )
{
	SourceAbiSlice bytes = { reinterpret_cast<const uint8_t *>( input ), inputLength };
	return source_compress_buffer_actual_size( bytes, size );
}

extern "C" SourceAbiStatus source_rust_bridge_buffer_decompress( const void *input,
	uint64_t inputLength, void *output, uint64_t capacity, uint64_t *length )
{
	SourceAbiSlice bytes = { reinterpret_cast<const uint8_t *>( input ), inputLength };
	return source_compress_buffer_decompress( bytes, reinterpret_cast<uint8_t *>( output ),
		capacity, length );
}

extern "C" SourceAbiStatus source_rust_bridge_split_packet_header_encode(
	int32_t sequence, uint32_t packetNumber, uint32_t packetCount, uint32_t splitSize,
	void *bytes )
{
	if ( bytes == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_net_split_packet_header_encode( sequence, packetNumber, packetCount,
		splitSize, reinterpret_cast<uint8_t *>( bytes ) );
}

extern "C" SourceAbiStatus source_rust_bridge_split_packet_header_decode(
	const void *datagram, uint64_t datagramLength, SourceAbiSplitPacketHeader *header )
{
	if ( header == 0 || ( datagramLength != 0 && datagram == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice datagramSlice;
	datagramSlice.data = reinterpret_cast<const uint8_t *>( datagram );
	datagramSlice.length = datagramLength;
	return source_net_split_packet_header_decode( datagramSlice, header );
}

extern "C" SourceAbiStatus source_rust_bridge_split_packet_create( uint64_t *peerId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || peerId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_split_packet_create( handle, peerId );
}

extern "C" SourceAbiStatus source_rust_bridge_split_packet_remove( uint64_t peerId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_split_packet_remove( handle, peerId );
}

extern "C" SourceAbiStatus source_rust_bridge_split_packet_reset( uint64_t peerId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_split_packet_reset( handle, peerId );
}

extern "C" SourceAbiStatus source_rust_bridge_split_packet_accept( uint64_t peerId,
	const void *datagram, uint64_t datagramLength, void *message, uint64_t capacity,
	uint64_t *length, bool *complete )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || length == 0 || complete == 0 ||
		( datagramLength != 0 && datagram == 0 ) || ( capacity != 0 && message == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice datagramSlice;
	datagramSlice.data = reinterpret_cast<const uint8_t *>( datagram );
	datagramSlice.length = datagramLength;
	uint32_t completeFlag = 0;
	const SourceAbiStatus status = source_context_split_packet_accept( handle, peerId,
		datagramSlice, reinterpret_cast<uint8_t *>( message ), capacity, length,
		&completeFlag );
	*complete = completeFlag != 0;
	return status;
}

extern "C" SourceAbiStatus source_rust_bridge_split_packet_collect( uint64_t peerId,
	void *message, uint64_t capacity, uint64_t *length )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || length == 0 || ( capacity != 0 && message == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_split_packet_collect( handle, peerId,
		reinterpret_cast<uint8_t *>( message ), capacity, length );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_create( const char *name,
	uint64_t nameLength, uint32_t maxEntries, int32_t tick, uint64_t *tableId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || ( nameLength != 0 && name == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice nameSlice;
	nameSlice.data = reinterpret_cast<const uint8_t *>( name );
	nameSlice.length = nameLength;
	return source_context_string_table_create( handle, nameSlice, maxEntries, tick, tableId );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_remove( uint64_t tableId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_string_table_remove( handle, tableId );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_clear( uint64_t tableId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_string_table_clear( handle, tableId );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_enable_history( uint64_t tableId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_string_table_enable_history( handle, tableId );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_set_tick( uint64_t tableId,
	int32_t tick )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_string_table_set_tick( handle, tableId, tick );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_synchronize_tick(
	uint64_t tableId, int32_t tick )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_string_table_synchronize_tick( handle, tableId, tick );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_upsert( uint64_t tableId,
	const char *value, uint64_t valueLength, bool updateUserData, const void *userData,
	uint64_t userDataLength, SourceAbiStringTableUpsert *result )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || result == 0 ||
		( valueLength != 0 && value == 0 ) ||
		( userDataLength != 0 && userData == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice valueSlice;
	valueSlice.data = reinterpret_cast<const uint8_t *>( value );
	valueSlice.length = valueLength;
	SourceAbiSlice userDataSlice;
	userDataSlice.data = static_cast<const uint8_t *>( userData );
	userDataSlice.length = userDataLength;
	return source_context_string_table_upsert( handle, tableId, valueSlice,
		updateUserData ? 1 : 0, userDataSlice, result );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_set_user_data(
	uint64_t tableId, uint32_t index, const void *userData, uint64_t userDataLength,
	SourceAbiStringTableChange *change )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || change == 0 ||
		( userDataLength != 0 && userData == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice userDataSlice;
	userDataSlice.data = static_cast<const uint8_t *>( userData );
	userDataSlice.length = userDataLength;
	return source_context_string_table_set_user_data( handle, tableId, index,
		userDataSlice, change );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_find( uint64_t tableId,
	const char *value, uint64_t valueLength, uint32_t *index )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || index == 0 ||
		( valueLength != 0 && value == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice valueSlice;
	valueSlice.data = reinterpret_cast<const uint8_t *>( value );
	valueSlice.length = valueLength;
	return source_context_string_table_find( handle, tableId, valueSlice, index );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_changed_since(
	uint64_t tableId, int32_t tick, uint32_t *changed )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || changed == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_string_table_changed_since( handle, tableId, tick, changed );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_restore_tick(
	uint64_t tableId, int32_t tick, int32_t *lastChangedTick )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || lastChangedTick == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_string_table_restore_tick( handle, tableId, tick,
		lastChangedTick );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_entry_count(
	uint64_t tableId, uint32_t *entryCount )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || entryCount == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_string_table_entry_count( handle, tableId, entryCount );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_encode_entries(
	uint64_t tableId, uint8_t *outBytes, uint64_t capacity, uint32_t *outBitCount )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || outBitCount == 0 ||
		( capacity != 0 && outBytes == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	return source_context_string_table_encode_entries( handle, tableId, outBytes,
		capacity, outBitCount );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_read_string( uint64_t tableId,
	uint32_t index, void *output, uint64_t outputLength, uint64_t *written )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || written == 0 ||
		( outputLength != 0 && output == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	return source_context_string_table_read_string( handle, tableId, index,
		outputSlice, written );
}

extern "C" SourceAbiStatus source_rust_bridge_string_table_read_user_data(
	uint64_t tableId, uint32_t index, void *output, uint64_t outputLength,
	uint64_t *written )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || tableId == 0 || written == 0 ||
		( outputLength != 0 && output == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	return source_context_string_table_read_user_data( handle, tableId, index,
		outputSlice, written );
}

extern "C" SourceAbiStatus source_rust_bridge_data_table_clear()
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_data_table_clear( handle );
}

extern "C" SourceAbiStatus source_rust_bridge_data_table_register( const char *name,
	uint64_t nameLength, uint32_t propertyCount,
	SourceAbiDataTableRegistration *registration )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || registration == 0 || ( nameLength != 0 && name == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice nameSlice;
	nameSlice.data = reinterpret_cast<const uint8_t *>( name );
	nameSlice.length = nameLength;
	return source_context_data_table_register( handle, nameSlice, propertyCount,
		registration );
}

extern "C" SourceAbiStatus source_rust_bridge_data_table_register_property(
	uint32_t tableId, const SourceAbiDataTableProperty *property )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || property == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_data_table_register_property( handle, tableId, property );
}

extern "C" SourceAbiStatus source_rust_bridge_server_class_register( const char *name,
	uint64_t nameLength, uint32_t tableId, uint32_t *classId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || classId == 0 || ( nameLength != 0 && name == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice nameSlice;
	nameSlice.data = reinterpret_cast<const uint8_t *>( name );
	nameSlice.length = nameLength;
	return source_context_server_class_register( handle, nameSlice, tableId, classId );
}

extern "C" SourceAbiStatus source_rust_bridge_data_table_finalize(
	uint32_t nativeCompatibilityCrc, SourceAbiDataTableSummary *summary )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || summary == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_data_table_finalize( handle, nativeCompatibilityCrc, summary );
}

extern "C" SourceAbiStatus source_rust_bridge_data_table_summary(
	SourceAbiDataTableSummary *summary )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || summary == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_data_table_summary( handle, summary );
}

extern "C" SourceAbiStatus source_rust_bridge_server_class_find( const char *name,
	uint64_t nameLength, uint32_t *classId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || classId == 0 || ( nameLength != 0 && name == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice nameSlice;
	nameSlice.data = reinterpret_cast<const uint8_t *>( name );
	nameSlice.length = nameLength;
	return source_context_server_class_find( handle, nameSlice, classId );
}

extern "C" SourceAbiStatus source_rust_bridge_snapshot_clear()
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_snapshot_clear( handle );
}

extern "C" SourceAbiStatus source_rust_bridge_snapshot_queue_delete( uint32_t slot,
	uint32_t *queued )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || queued == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_snapshot_queue_delete( handle, slot, queued );
}

extern "C" SourceAbiStatus source_rust_bridge_snapshot_create( int32_t tick,
	uint32_t maxEntities, const SourceAbiSnapshotEntity *entities, uint64_t entityCount,
	uint64_t *snapshotId, SourceAbiSnapshotSummary *summary )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || snapshotId == 0 || summary == 0 ||
		( entityCount != 0 && entities == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	return source_context_snapshot_create( handle, tick, maxEntities, entities,
		entityCount, snapshotId, summary );
}

extern "C" SourceAbiStatus source_rust_bridge_snapshot_remove( uint64_t snapshotId )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || snapshotId == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_snapshot_remove( handle, snapshotId );
}

extern "C" SourceAbiStatus source_rust_bridge_snapshot_summary( uint64_t snapshotId,
	SourceAbiSnapshotSummary *summary )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || snapshotId == 0 || summary == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_snapshot_summary( handle, snapshotId, summary );
}

extern "C" SourceAbiStatus source_rust_bridge_snapshot_entity_at( uint64_t snapshotId,
	uint32_t ordinal, SourceAbiSnapshotEntity *entity )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || snapshotId == 0 || entity == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_snapshot_entity_at( handle, snapshotId, ordinal, entity );
}

extern "C" SourceAbiStatus source_rust_bridge_snapshot_delete_at( uint64_t snapshotId,
	uint32_t ordinal, uint32_t *slot )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || snapshotId == 0 || slot == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_snapshot_delete_at( handle, snapshotId, ordinal, slot );
}

extern "C" SourceAbiStatus source_rust_bridge_snapshot_delta( uint64_t fromSnapshotId,
	const uint32_t *fromVisible, uint64_t fromVisibleCount, uint64_t toSnapshotId,
	const uint32_t *toVisible, uint64_t toVisibleCount, SourceAbiSnapshotDelta *deltas,
	uint64_t capacity, uint64_t *count )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || toSnapshotId == 0 || count == 0 ||
		( capacity != 0 && deltas == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_snapshot_delta( handle, fromSnapshotId, fromVisible,
		fromVisibleCount, toSnapshotId, toVisible, toVisibleCount, deltas, capacity,
		count );
}

// Borrows a null-terminated string as an ABI slice. The callee only reads it
// during the call, so the caller's storage stays valid for its whole lifetime.
static SourceAbiSlice MakeSlice( const char *text )
{
	SourceAbiSlice slice;
	slice.data = reinterpret_cast<const uint8_t *>( text );
	slice.length = strlen( text );
	return slice;
}

extern "C" SourceAbiStatus source_rust_bridge_demo_header_encode( int32_t demoProtocol,
	int32_t networkProtocol, const char *serverName, const char *clientName,
	const char *mapName, const char *gameDirectory, float playbackTime,
	int32_t playbackTicks, int32_t playbackFrames, int32_t signonLength, uint8_t *bytes,
	uint64_t capacity, uint64_t *size )
{
	// The encoder holds no state, so it needs no active context handle.
	if ( serverName == 0 || clientName == 0 || mapName == 0 || gameDirectory == 0 ||
		size == 0 || ( capacity != 0 && bytes == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_demo_header_encode( demoProtocol, networkProtocol,
		MakeSlice( serverName ), MakeSlice( clientName ), MakeSlice( mapName ),
		MakeSlice( gameDirectory ), playbackTime, playbackTicks, playbackFrames,
		signonLength, bytes, capacity, size );
}

extern "C" SourceAbiStatus source_rust_bridge_delta_header_encode( uint32_t entityIndex,
	int32_t headerBase, bool leavePvs, bool deleteEntity, bool enterPvs, uint8_t *bytes,
	uint64_t capacity, uint32_t *bitCount )
{
	// The encoder holds no state, so it needs no active context handle.
	if ( bitCount == 0 || ( capacity != 0 && bytes == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_delta_header_encode( entityIndex, headerBase, leavePvs ? 1u : 0u,
		deleteEntity ? 1u : 0u, enterPvs ? 1u : 0u, bytes, capacity, bitCount );
}

extern "C" SourceAbiStatus source_rust_bridge_command_enqueue( const char *script,
	uint64_t scriptLength, uint32_t *commandCount )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice scriptSlice;
	scriptSlice.data = reinterpret_cast<const uint8_t *>( script );
	scriptSlice.length = scriptLength;
	return source_context_command_enqueue( handle, scriptSlice, commandCount );
}

extern "C" SourceAbiStatus source_rust_bridge_command_pop( void *output,
	uint64_t outputLength, uint64_t *written )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>( output );
	outputSlice.length = outputLength;
	return source_context_command_pop( handle, outputSlice, written );
}

extern "C" SourceAbiStatus source_rust_bridge_demo_validate( const char *virtualPath,
	uint64_t virtualPathLength, SourceAbiDemoInfo *info )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || virtualPath == 0 || info == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	static const char pathId[] = "GAME";
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = sizeof( pathId ) - 1;
	return source_context_demo_validate( handle, virtualPathSlice, pathIdSlice, info );
}

extern "C" SourceAbiStatus source_rust_bridge_save_validate( const char *virtualPath,
	uint64_t virtualPathLength, SourceAbiSaveInfo *info )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || virtualPath == 0 || info == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>( virtualPath );
	virtualPathSlice.length = virtualPathLength;
	static const char pathId[] = "GAME";
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>( pathId );
	pathIdSlice.length = sizeof( pathId ) - 1;
	return source_context_save_validate( handle, virtualPathSlice, pathIdSlice, info );
}

extern "C" SourceAbiStatus source_rust_bridge_save_container_header_encode(
	const void *tokens, uint64_t tokensLength, uint64_t tokenCount,
	const void *gameData, uint64_t gameDataLength, void *outBytes,
	uint64_t capacity, uint64_t *outSize )
{
	if ( outSize == 0 || ( capacity != 0 && outBytes == 0 ) ||
		( tokensLength != 0 && tokens == 0 ) ||
		( gameDataLength != 0 && gameData == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice tokensSlice;
	tokensSlice.data = reinterpret_cast<const uint8_t *>( tokens );
	tokensSlice.length = tokensLength;
	SourceAbiSlice gameDataSlice;
	gameDataSlice.data = reinterpret_cast<const uint8_t *>( gameData );
	gameDataSlice.length = gameDataLength;
	return source_save_container_header_encode( tokensSlice, tokenCount,
		gameDataSlice, reinterpret_cast<uint8_t *>( outBytes ), capacity, outSize );
}

extern "C" SourceAbiStatus source_rust_bridge_save_map_state_encode(
	const void *tokens, uint64_t tokensLength, uint64_t tokenCount,
	const void *dataHeaders, uint64_t dataHeadersLength, const void *data,
	uint64_t dataLength, void *outBytes, uint64_t capacity, uint64_t *outSize )
{
	// Composing a container needs no engine context, so this stays callable
	// during the save path regardless of runtime handle state.
	if ( outSize == 0 || ( capacity != 0 && outBytes == 0 ) ||
		( tokensLength != 0 && tokens == 0 ) ||
		( dataHeadersLength != 0 && dataHeaders == 0 ) ||
		( dataLength != 0 && data == 0 ) )
	{
		return SOURCE_ABI_INVALID_ARGUMENT;
	}
	SourceAbiSlice tokensSlice;
	tokensSlice.data = reinterpret_cast<const uint8_t *>( tokens );
	tokensSlice.length = tokensLength;
	SourceAbiSlice dataHeadersSlice;
	dataHeadersSlice.data = reinterpret_cast<const uint8_t *>( dataHeaders );
	dataHeadersSlice.length = dataHeadersLength;
	SourceAbiSlice dataSlice;
	dataSlice.data = reinterpret_cast<const uint8_t *>( data );
	dataSlice.length = dataLength;
	return source_save_map_state_encode( tokensSlice, tokenCount, dataHeadersSlice,
		dataSlice, reinterpret_cast<uint8_t *>( outBytes ), capacity, outSize );
}

extern "C" SourceAbiStatus source_rust_bridge_host_run_frames(
	SourceAbiFrameFn runFrame, void *userData, uint64_t maxIterations,
	SourceAbiFrameLoopInfo *info )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_host_run_frames( handle, runFrame, userData,
		maxIterations, info );
}

extern "C" SourceAbiStatus source_rust_bridge_host_pace( uint64_t nowNs,
	uint64_t minimumFrameNs, SourceAbiFramePace *pace )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || pace == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_host_pace( handle, nowNs, minimumFrameNs, pace );
}

extern "C" SourceAbiStatus source_rust_bridge_host_schedule_ticks( double frameTime,
	double tickInterval, int32_t startTick, bool accumulate, bool alternateTicks,
	SourceAbiTickPlan *plan )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || plan == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_host_schedule_ticks( handle, frameTime, tickInterval,
		startTick, accumulate ? 1 : 0, alternateTicks ? 1 : 0, plan );
}

extern "C" SourceAbiStatus source_rust_bridge_host_request_operation( uint32_t kind,
	const char *target, uint64_t targetLength, const char *landmark,
	uint64_t landmarkLength, uint32_t flags )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || ( targetLength != 0 && target == 0 ) ||
		( landmarkLength != 0 && landmark == 0 ) )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiSlice targetSlice;
	targetSlice.data = reinterpret_cast<const uint8_t *>( target );
	targetSlice.length = targetLength;
	SourceAbiSlice landmarkSlice;
	landmarkSlice.data = reinterpret_cast<const uint8_t *>( landmark );
	landmarkSlice.length = landmarkLength;
	return source_context_host_request_operation( handle, kind, targetSlice,
		landmarkSlice, flags );
}

extern "C" SourceAbiStatus source_rust_bridge_host_take_operation( void *targetOutput,
	uint64_t targetOutputLength, void *landmarkOutput, uint64_t landmarkOutputLength,
	SourceAbiHostOperationInfo *info )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 || info == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	SourceAbiMutSlice targetSlice;
	targetSlice.data = static_cast<uint8_t *>( targetOutput );
	targetSlice.length = targetOutputLength;
	SourceAbiMutSlice landmarkSlice;
	landmarkSlice.data = static_cast<uint8_t *>( landmarkOutput );
	landmarkSlice.length = landmarkOutputLength;
	return source_context_host_take_operation( handle, targetSlice, landmarkSlice, info );
}

extern "C" SourceAbiStatus source_rust_bridge_host_pending_operation( uint32_t *kind )
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_host_pending_operation( handle, kind );
}

extern "C" SourceAbiStatus source_rust_bridge_host_clear_operation()
{
	const SourceAbiHandle handle = g_ActiveRustEngineHandle.load( std::memory_order_acquire );
	if ( handle == 0 )
		return SOURCE_ABI_INVALID_ARGUMENT;
	return source_context_host_clear_operation( handle );
}

CRustEngineBridge::CRustEngineBridge() : m_Handle(0)
{
}

CRustEngineBridge::~CRustEngineBridge()
{
	Shutdown();
}

SourceAbiStatus CRustEngineBridge::Init(SourceAbiLogFn log, void *userData)
{
	if (m_Handle != 0)
		return SOURCE_ABI_INVALID_ARGUMENT;

	SourceAbiContextConfig config;
	config.struct_size = sizeof(config);
	config.abi_version = SOURCE_ABI_VERSION;
	config.log = log;
	config.user_data = userData;
	return source_context_create(&config, &m_Handle);
}

SourceAbiStatus CRustEngineBridge::Shutdown()
{
	if (m_Handle == 0)
		return SOURCE_ABI_OK;

	const SourceAbiHandle handle = m_Handle;
	source_rust_bridge_deactivate( handle );
	m_Handle = 0;
	return source_context_destroy(handle);
}

SourceAbiStatus CRustEngineBridge::Echo(const void *input, uint64_t inputLength,
	void *output, uint64_t outputLength, uint64_t *written) const
{
	SourceAbiSlice inputSlice;
	inputSlice.data = static_cast<const uint8_t *>(input);
	inputSlice.length = inputLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>(output);
	outputSlice.length = outputLength;
	return source_context_echo(m_Handle, inputSlice, outputSlice, written);
}

SourceAbiStatus CRustEngineBridge::EmitLog(int32_t level, const char *message,
	uint64_t length) const
{
	SourceAbiSlice messageSlice;
	messageSlice.data = reinterpret_cast<const uint8_t *>(message);
	messageSlice.length = length;
	return source_context_emit_log(m_Handle, level, messageSlice);
}

SourceAbiStatus CRustEngineBridge::ResolveContentPath(const char *root, uint64_t rootLength,
	const char *virtualPath, uint64_t virtualPathLength, void *output,
	uint64_t outputLength, uint64_t *written) const
{
	SourceAbiSlice rootSlice;
	rootSlice.data = reinterpret_cast<const uint8_t *>(root);
	rootSlice.length = rootLength;
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>(virtualPath);
	virtualPathSlice.length = virtualPathLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>(output);
	outputSlice.length = outputLength;
	return source_context_resolve_content_path(m_Handle, rootSlice, virtualPathSlice,
		outputSlice, written);
}

SourceAbiStatus CRustEngineBridge::ExecutableBase(void *output, uint64_t outputLength,
	uint64_t *written) const
{
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>(output);
	outputSlice.length = outputLength;
	return source_context_executable_base(m_Handle, outputSlice, written);
}

SourceAbiStatus CRustEngineBridge::MountGameInfo(const char *base, uint64_t baseLength,
	const char *game, uint64_t gameLength, const char *externalRoot,
	uint64_t externalRootLength, uint64_t *mountCount) const
{
	SourceAbiSlice baseSlice = { reinterpret_cast<const uint8_t *>(base), baseLength };
	SourceAbiSlice gameSlice = { reinterpret_cast<const uint8_t *>(game), gameLength };
	SourceAbiSlice externalSlice = { reinterpret_cast<const uint8_t *>(externalRoot), externalRootLength };
	return source_context_mount_gameinfo(m_Handle, baseSlice, gameSlice, externalSlice, mountCount);
}

SourceAbiStatus CRustEngineBridge::MountDirectory(const char *root, uint64_t rootLength,
	const char *pathId, uint64_t pathIdLength, bool atHead) const
{
	SourceAbiSlice rootSlice;
	rootSlice.data = reinterpret_cast<const uint8_t *>(root);
	rootSlice.length = rootLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>(pathId);
	pathIdSlice.length = pathIdLength;
	return source_context_mount_directory(m_Handle, rootSlice, pathIdSlice, atHead ? 1 : 0);
}

SourceAbiStatus CRustEngineBridge::MountVpk(const char *directoryPath,
	uint64_t directoryPathLength, const char *pathId, uint64_t pathIdLength,
	bool atHead) const
{
	SourceAbiSlice directoryPathSlice;
	directoryPathSlice.data = reinterpret_cast<const uint8_t *>(directoryPath);
	directoryPathSlice.length = directoryPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>(pathId);
	pathIdSlice.length = pathIdLength;
	return source_context_mount_vpk(m_Handle, directoryPathSlice, pathIdSlice,
		atHead ? 1 : 0);
}

SourceAbiStatus CRustEngineBridge::ReadFile(const char *virtualPath,
	uint64_t virtualPathLength, const char *pathId, uint64_t pathIdLength,
	void *output, uint64_t outputLength, uint64_t *written) const
{
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>(virtualPath);
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>(pathId);
	pathIdSlice.length = pathIdLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>(output);
	outputSlice.length = outputLength;
	return source_context_read_file(m_Handle, virtualPathSlice, pathIdSlice,
		outputSlice, written);
}

SourceAbiStatus CRustEngineBridge::ProbeScene(const char *virtualPath,
	uint64_t virtualPathLength, const char *pathId, uint64_t pathIdLength,
	uint64_t *sceneCount, uint64_t *stringCount) const
{
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>(virtualPath);
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>(pathId);
	pathIdSlice.length = pathIdLength;
	return source_context_probe_scene(m_Handle, virtualPathSlice, pathIdSlice,
		sceneCount, stringCount);
}

SourceAbiStatus CRustEngineBridge::DemoValidate(const char *virtualPath,
	uint64_t virtualPathLength, const char *pathId, uint64_t pathIdLength,
	SourceAbiDemoInfo *info) const
{
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>(virtualPath);
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>(pathId);
	pathIdSlice.length = pathIdLength;
	return source_context_demo_validate(m_Handle, virtualPathSlice, pathIdSlice, info);
}

SourceAbiStatus CRustEngineBridge::SaveValidate(const char *virtualPath,
	uint64_t virtualPathLength, const char *pathId, uint64_t pathIdLength,
	SourceAbiSaveInfo *info) const
{
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>(virtualPath);
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>(pathId);
	pathIdSlice.length = pathIdLength;
	return source_context_save_validate(m_Handle, virtualPathSlice, pathIdSlice, info);
}

SourceAbiStatus CRustEngineBridge::WorldLoad(const char *virtualPath,
	uint64_t virtualPathLength, const char *pathId, uint64_t pathIdLength,
	SourceAbiWorldInfo *info) const
{
	SourceAbiSlice virtualPathSlice;
	virtualPathSlice.data = reinterpret_cast<const uint8_t *>(virtualPath);
	virtualPathSlice.length = virtualPathLength;
	SourceAbiSlice pathIdSlice;
	pathIdSlice.data = reinterpret_cast<const uint8_t *>(pathId);
	pathIdSlice.length = pathIdLength;
	return source_context_world_load(m_Handle, virtualPathSlice, pathIdSlice, info);
}

SourceAbiStatus CRustEngineBridge::WorldClear() const
{
	return source_context_world_clear(m_Handle);
}

SourceAbiStatus CRustEngineBridge::WorldPointLeaf(float x, float y, float z,
	SourceAbiWorldLeaf *leaf) const
{
	return source_context_world_point_leaf(m_Handle, x, y, z, leaf);
}

SourceAbiStatus CRustEngineBridge::WorldVisibility(uint32_t cluster, uint32_t kind,
	void *output, uint64_t outputLength, uint64_t *written) const
{
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>(output);
	outputSlice.length = outputLength;
	return source_context_world_visibility(m_Handle, cluster, kind, outputSlice, written);
}

SourceAbiStatus CRustEngineBridge::HostTransition(uint32_t phase) const
{
	return source_context_host_transition(m_Handle, phase);
}

SourceAbiStatus CRustEngineBridge::HostPhase(uint32_t *phase) const
{
	return source_context_host_phase(m_Handle, phase);
}

SourceAbiStatus CRustEngineBridge::HostConfigureScheduler(uint64_t tickIntervalNs,
	uint32_t maxCatchUpTicks) const
{
	return source_context_host_configure_scheduler(m_Handle, tickIntervalNs,
		maxCatchUpTicks);
}

SourceAbiStatus CRustEngineBridge::HostAdvance(uint64_t nowNs, SourceAbiFramePlan *plan) const
{
	return source_context_host_advance(m_Handle, nowNs, plan);
}

SourceAbiStatus CRustEngineBridge::HostPace(uint64_t nowNs,
	uint64_t minimumFrameNs, SourceAbiFramePace *pace) const
{
	return source_context_host_pace(m_Handle, nowNs, minimumFrameNs, pace);
}

SourceAbiStatus CRustEngineBridge::HostScheduleTicks(double frameTime,
	double tickInterval, int32_t startTick, bool accumulate, bool alternateTicks,
	SourceAbiTickPlan *plan) const
{
	return source_context_host_schedule_ticks(m_Handle, frameTime, tickInterval,
		startTick, accumulate ? 1 : 0, alternateTicks ? 1 : 0, plan);
}

SourceAbiStatus CRustEngineBridge::HostRequestOperation(uint32_t kind,
	const char *target, uint64_t targetLength, const char *landmark,
	uint64_t landmarkLength, uint32_t flags) const
{
	SourceAbiSlice targetSlice;
	targetSlice.data = reinterpret_cast<const uint8_t *>(target);
	targetSlice.length = targetLength;
	SourceAbiSlice landmarkSlice;
	landmarkSlice.data = reinterpret_cast<const uint8_t *>(landmark);
	landmarkSlice.length = landmarkLength;
	return source_context_host_request_operation(m_Handle, kind, targetSlice,
		landmarkSlice, flags);
}

SourceAbiStatus CRustEngineBridge::HostTakeOperation(void *targetOutput,
	uint64_t targetOutputLength, void *landmarkOutput, uint64_t landmarkOutputLength,
	SourceAbiHostOperationInfo *info) const
{
	SourceAbiMutSlice targetSlice;
	targetSlice.data = static_cast<uint8_t *>(targetOutput);
	targetSlice.length = targetOutputLength;
	SourceAbiMutSlice landmarkSlice;
	landmarkSlice.data = static_cast<uint8_t *>(landmarkOutput);
	landmarkSlice.length = landmarkOutputLength;
	return source_context_host_take_operation(m_Handle, targetSlice, landmarkSlice, info);
}

SourceAbiStatus CRustEngineBridge::HostPendingOperation(uint32_t *kind) const
{
	return source_context_host_pending_operation(m_Handle, kind);
}

SourceAbiStatus CRustEngineBridge::HostClearOperation() const
{
	return source_context_host_clear_operation(m_Handle);
}

SourceAbiStatus CRustEngineBridge::HostRunSessions(SourceAbiSessionFn runSession,
	void *userData, uint32_t maxSessions, uint32_t *sessionCount) const
{
	return source_context_host_run_sessions(m_Handle, runSession, userData,
		maxSessions, sessionCount);
}

SourceAbiStatus CRustEngineBridge::HostRunFrames(SourceAbiFrameFn runFrame,
	void *userData, uint64_t maxIterations, SourceAbiFrameLoopInfo *info) const
{
	return source_context_host_run_frames(m_Handle, runFrame, userData,
		maxIterations, info);
}

SourceAbiStatus CRustEngineBridge::CvarRegister(const char *name, uint64_t nameLength,
	const char *defaultValue, uint64_t defaultValueLength, uint32_t flags) const
{
	SourceAbiSlice nameSlice;
	nameSlice.data = reinterpret_cast<const uint8_t *>(name);
	nameSlice.length = nameLength;
	SourceAbiSlice defaultValueSlice;
	defaultValueSlice.data = reinterpret_cast<const uint8_t *>(defaultValue);
	defaultValueSlice.length = defaultValueLength;
	return source_context_cvar_register(m_Handle, nameSlice, defaultValueSlice, flags);
}

SourceAbiStatus CRustEngineBridge::CvarSet(const char *name, uint64_t nameLength,
	const char *value, uint64_t valueLength) const
{
	SourceAbiSlice nameSlice;
	nameSlice.data = reinterpret_cast<const uint8_t *>(name);
	nameSlice.length = nameLength;
	SourceAbiSlice valueSlice;
	valueSlice.data = reinterpret_cast<const uint8_t *>(value);
	valueSlice.length = valueLength;
	return source_context_cvar_set(m_Handle, nameSlice, valueSlice);
}

SourceAbiStatus CRustEngineBridge::CvarGet(const char *name, uint64_t nameLength,
	void *output, uint64_t outputLength, uint64_t *written, SourceAbiCvarInfo *info) const
{
	SourceAbiSlice nameSlice;
	nameSlice.data = reinterpret_cast<const uint8_t *>(name);
	nameSlice.length = nameLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>(output);
	outputSlice.length = outputLength;
	return source_context_cvar_get(m_Handle, nameSlice, outputSlice, written, info);
}

SourceAbiStatus CRustEngineBridge::CommandEnqueue(const char *script,
	uint64_t scriptLength, uint32_t *commandCount) const
{
	SourceAbiSlice scriptSlice;
	scriptSlice.data = reinterpret_cast<const uint8_t *>(script);
	scriptSlice.length = scriptLength;
	return source_context_command_enqueue(m_Handle, scriptSlice, commandCount);
}

SourceAbiStatus CRustEngineBridge::CommandPop(void *output, uint64_t outputLength,
	uint64_t *written) const
{
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>(output);
	outputSlice.length = outputLength;
	return source_context_command_pop(m_Handle, outputSlice, written);
}

SourceAbiStatus CRustEngineBridge::InputModifier(uint32_t modifier, bool pressed,
	uint32_t *modifierMask) const
{
	return source_context_input_modifier(m_Handle, modifier, pressed ? 1 : 0,
		modifierMask);
}

SourceAbiStatus CRustEngineBridge::InputMouseButton(uint32_t button, bool pressed,
	uint32_t *buttonMask) const
{
	return source_context_input_mouse_button(m_Handle, button, pressed ? 1 : 0,
		buttonMask);
}

SourceAbiStatus CRustEngineBridge::InputMouseMotion(int32_t deltaX, int32_t deltaY) const
{
	return source_context_input_mouse_motion(m_Handle, deltaX, deltaY);
}

SourceAbiStatus CRustEngineBridge::InputTakeMouseDelta(int32_t *deltaX, int32_t *deltaY) const
{
	return source_context_input_take_mouse_delta(m_Handle, deltaX, deltaY);
}

SourceAbiStatus CRustEngineBridge::InputReset() const
{
	return source_context_input_reset(m_Handle);
}

SourceAbiStatus CRustEngineBridge::ProbeVpk(const char *path, uint64_t pathLength,
	uint64_t *entryCount, uint32_t *version) const
{
	SourceAbiSlice pathSlice;
	pathSlice.data = reinterpret_cast<const uint8_t *>(path);
	pathSlice.length = pathLength;
	return source_context_probe_vpk(m_Handle, pathSlice, entryCount, version);
}

SourceAbiStatus CRustEngineBridge::ReadVpkFile(const char *directoryPath,
	uint64_t directoryPathLength, const char *entryPath, uint64_t entryPathLength,
	void *output, uint64_t outputLength, uint64_t *written) const
{
	SourceAbiSlice directoryPathSlice;
	directoryPathSlice.data = reinterpret_cast<const uint8_t *>(directoryPath);
	directoryPathSlice.length = directoryPathLength;
	SourceAbiSlice entryPathSlice;
	entryPathSlice.data = reinterpret_cast<const uint8_t *>(entryPath);
	entryPathSlice.length = entryPathLength;
	SourceAbiMutSlice outputSlice;
	outputSlice.data = static_cast<uint8_t *>(output);
	outputSlice.length = outputLength;
	return source_context_read_vpk_file(m_Handle, directoryPathSlice, entryPathSlice,
		outputSlice, written);
}

SourceAbiStatus CRustEngineBridge::ProbeVpkScene(const char *directoryPath,
	uint64_t directoryPathLength, const char *entryPath, uint64_t entryPathLength,
	uint64_t *sceneCount, uint64_t *stringCount) const
{
	SourceAbiSlice directoryPathSlice;
	directoryPathSlice.data = reinterpret_cast<const uint8_t *>(directoryPath);
	directoryPathSlice.length = directoryPathLength;
	SourceAbiSlice entryPathSlice;
	entryPathSlice.data = reinterpret_cast<const uint8_t *>(entryPath);
	entryPathSlice.length = entryPathLength;
	return source_context_probe_vpk_scene(m_Handle, directoryPathSlice, entryPathSlice,
		sceneCount, stringCount);
}

extern "C" bool source_rust_bridge_ui_active()
{
	return g_MetalPresenterHandle.load( std::memory_order_acquire ) != 0;
}

extern "C" SourceAbiStatus source_rust_bridge_ui_begin()
{
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( presenter == 0 )
		return SOURCE_ABI_INVALID_HANDLE;
	return source_render_ui_begin( presenter );
}

extern "C" SourceAbiStatus source_rust_bridge_ui_texture(
	uint32_t id, uint32_t width, uint32_t height, const uint8_t *rgba )
{
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( presenter == 0 || rgba == NULL )
		return SOURCE_ABI_INVALID_HANDLE;
	return source_render_ui_texture( presenter, id, width, height, rgba );
}

extern "C" SourceAbiStatus source_rust_bridge_ui_texture_region(
	uint32_t id, uint32_t x, uint32_t y, uint32_t width, uint32_t height,
	const uint8_t *rgba )
{
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( presenter == 0 || rgba == NULL )
		return SOURCE_ABI_INVALID_HANDLE;
	return source_render_ui_texture_region( presenter, id, x, y, width, height, rgba );
}

extern "C" SourceAbiStatus source_rust_bridge_ui_texture_alias( uint32_t alias, uint32_t base )
{
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( presenter == 0 )
		return SOURCE_ABI_INVALID_HANDLE;
	return source_render_ui_texture_alias( presenter, alias, base );
}

extern "C" bool source_rust_bridge_ui_has_texture( uint32_t id )
{
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( presenter == 0 )
		return false;
	return source_render_ui_has_texture( presenter, id ) != 0;
}

extern "C" SourceAbiStatus source_rust_bridge_ui_quad(
	uint32_t texture, const float *bounds, const float *coords, const float *tint )
{
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( presenter == 0 )
		return SOURCE_ABI_INVALID_HANDLE;
	return source_render_ui_quad( presenter, texture, bounds, coords, tint );
}

extern "C" SourceAbiStatus source_rust_bridge_ui_end(
	uint32_t width, uint32_t height, uint64_t *quads )
{
	const SourceAbiHandle presenter = g_MetalPresenterHandle.load( std::memory_order_acquire );
	if ( presenter == 0 )
		return SOURCE_ABI_INVALID_HANDLE;
	return source_render_ui_end( presenter, width, height, quads );
}

// The Direct3D 9 on Metal device is called from tometal, not from here, and a
// static library only contributes the objects something refers to. Naming the
// entry points keeps them in this dylib for tometal to link against.
extern "C" const void *const g_SourceD3D9Exports[] =
{
	(const void *)&source_d3d9_set_window,
	(const void *)&source_d3d9_display_info,
	(const void *)&source_d3d9_format_supported,
	(const void *)&source_d3d9_device_create,
	(const void *)&source_d3d9_device_destroy,
	(const void *)&source_d3d9_device_reset,
	(const void *)&source_d3d9_texture_create,
	(const void *)&source_d3d9_texture_destroy,
	(const void *)&source_d3d9_texture_lock,
	(const void *)&source_d3d9_texture_unlock,
	(const void *)&source_d3d9_buffer_create,
	(const void *)&source_d3d9_buffer_destroy,
	(const void *)&source_d3d9_buffer_lock,
	(const void *)&source_d3d9_buffer_unlock,
	(const void *)&source_d3d9_vertex_declaration_create,
	(const void *)&source_d3d9_vertex_declaration_destroy,
	(const void *)&source_d3d9_shader_create,
	(const void *)&source_d3d9_shader_destroy,
	(const void *)&source_d3d9_draw,
	(const void *)&source_d3d9_draw_indexed,
	(const void *)&source_d3d9_clear,
	(const void *)&source_d3d9_stretch_rect,
	(const void *)&source_d3d9_read_render_target,
	(const void *)&source_d3d9_present,
	(const void *)&source_d3d9_query_create,
	(const void *)&source_d3d9_query_destroy,
	(const void *)&source_d3d9_query_issue,
	(const void *)&source_d3d9_query_get_data,
};
