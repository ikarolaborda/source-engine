/* Stands in for the engine's allocator symbol, which tier1/lzss.cpp
 * references because its memory-override header declares it.
 *
 * The differential harness only calls CompressNoAlloc and SafeUncompress, and
 * both write into buffers the caller already owns, so nothing reachable from
 * the comparison dereferences this. It exists so the codec can be linked on
 * its own, without pulling in tier0 and the allocator it installs.
 *
 * Kept in C so it defines the symbol by that exact name without needing the
 * engine's headers, and therefore without needing to agree with them on what
 * it points at. */
void *g_pMemAlloc = 0;
