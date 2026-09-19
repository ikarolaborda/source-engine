#ifndef SOURCE_D3D9_H
#define SOURCE_D3D9_H

// The C ABI between the Direct3D 9 classes in tometal/ and the Metal device in
// rust/crates/source-d3d9. See docs/rust-port/d3d9-metal.md.
//
// The Direct3D state machine lives on the C++ side as one plain struct that a
// draw hands across by pointer, because the engine sets state tens of
// thousands of times a frame and draws a few thousand: a Set* call is an array
// write, and only draws, clears, copies and resource traffic cross into Rust.
//
// Handles are opaque and owned by the C++ object that holds them. The C++ side
// scrubs a handle out of the state block before destroying it, so the state
// block never names a dead object.

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SOURCE_D3D9_RENDER_STATES 256
#define SOURCE_D3D9_SAMPLERS 16
#define SOURCE_D3D9_SAMPLER_STATES 16
#define SOURCE_D3D9_STREAMS 8
#define SOURCE_D3D9_RENDER_TARGETS 4
#define SOURCE_D3D9_VS_FLOAT_CONSTANTS 256
#define SOURCE_D3D9_PS_FLOAT_CONSTANTS 224
#define SOURCE_D3D9_INT_CONSTANTS 16

typedef uint64_t SourceD3D9Handle;

// One face and mip of a texture, which is what Direct3D calls a surface.
typedef struct SourceD3D9SurfaceRef
{
	SourceD3D9Handle texture;
	uint32_t face;
	uint32_t level;
} SourceD3D9SurfaceRef;

typedef struct SourceD3D9Stream
{
	SourceD3D9Handle buffer;
	uint32_t offset;
	uint32_t stride;
} SourceD3D9Stream;

typedef struct SourceD3D9Viewport
{
	uint32_t x;
	uint32_t y;
	uint32_t width;
	uint32_t height;
	float min_z;
	float max_z;
} SourceD3D9Viewport;

typedef struct SourceD3D9Rect
{
	int32_t left;
	int32_t top;
	int32_t right;
	int32_t bottom;
} SourceD3D9Rect;

typedef struct SourceD3D9State
{
	uint32_t render_states[SOURCE_D3D9_RENDER_STATES];				// indexed by D3DRENDERSTATETYPE
	uint32_t sampler_states[SOURCE_D3D9_SAMPLERS][SOURCE_D3D9_SAMPLER_STATES];	// indexed by D3DSAMPLERSTATETYPE
	SourceD3D9Handle textures[SOURCE_D3D9_SAMPLERS];
	SourceD3D9SurfaceRef render_targets[SOURCE_D3D9_RENDER_TARGETS];
	SourceD3D9SurfaceRef depth_stencil;
	SourceD3D9Handle vertex_shader;
	SourceD3D9Handle pixel_shader;
	SourceD3D9Handle vertex_declaration;
	SourceD3D9Stream streams[SOURCE_D3D9_STREAMS];
	SourceD3D9Handle index_buffer;
	SourceD3D9Viewport viewport;
	SourceD3D9Rect scissor;
	float clip_plane0[4];
	uint32_t vs_bools;
	uint32_t ps_bools;
	int32_t vs_ints[SOURCE_D3D9_INT_CONSTANTS][4];
	int32_t ps_ints[SOURCE_D3D9_INT_CONSTANTS][4];
	float vs_floats[SOURCE_D3D9_VS_FLOAT_CONSTANTS][4];
	float ps_floats[SOURCE_D3D9_PS_FLOAT_CONSTANTS][4];
} SourceD3D9State;

typedef struct SourceD3D9VertexElement
{
	uint16_t stream;
	uint16_t offset;
	uint8_t type;			// D3DDECLTYPE
	uint8_t method;
	uint8_t usage;			// D3DDECLUSAGE
	uint8_t usage_index;
} SourceD3D9VertexElement;

// What the display can show, in pixels rather than points.
typedef struct SourceD3D9DisplayInfo
{
	uint32_t pixel_width;
	uint32_t pixel_height;
	uint32_t refresh_hz;
	float backing_scale;
	uint64_t recommended_memory;
	char name[128];
} SourceD3D9DisplayInfo;

enum
{
	SOURCE_D3D9_TEXTURE_2D = 0,
	SOURCE_D3D9_TEXTURE_CUBE = 1,
	SOURCE_D3D9_TEXTURE_VOLUME = 2,
};

enum
{
	SOURCE_D3D9_SHADER_VERTEX = 0,
	SOURCE_D3D9_SHADER_PIXEL = 1,
};

// The window the device presents into. Called by the window manager, on the
// main thread, before the device exists. `ns_window` is an NSWindow.
void source_d3d9_set_window( void *ns_window );

// Fills in the main display's description. Returns nonzero on success.
int32_t source_d3d9_display_info( SourceD3D9DisplayInfo *info );

// Nonzero when the device can create a texture of this D3DFORMAT. `usage` is
// the D3DUSAGE_* mask the caller wants and `type` a SOURCE_D3D9_TEXTURE_*.
int32_t source_d3d9_format_supported( uint32_t d3d_format, uint32_t usage, uint32_t type );

// Creates the device and its back buffer. Returns zero on failure.
SourceD3D9Handle source_d3d9_device_create( uint32_t width, uint32_t height );
void source_d3d9_device_destroy( SourceD3D9Handle device );

// Resizes what is presented. The back buffer surfaces themselves are textures
// the C++ side creates and hands to `present`.
void source_d3d9_device_reset( SourceD3D9Handle device, uint32_t width, uint32_t height );

SourceD3D9Handle source_d3d9_texture_create(
	SourceD3D9Handle device,
	uint32_t type,
	uint32_t width,
	uint32_t height,
	uint32_t depth,
	uint32_t levels,
	uint32_t usage,
	uint32_t d3d_format,
	const char *debug_label );
void source_d3d9_texture_destroy( SourceD3D9Handle device, SourceD3D9Handle texture );

// Locks a rectangle (a box, for a volume) of one face and level for writing,
// or for reading back when `readback` is nonzero. `pitch` is the byte stride
// between rows of the returned memory and `slice_pitch` between depth slices.
// Returns zero on failure.
int32_t source_d3d9_texture_lock(
	SourceD3D9Handle device,
	SourceD3D9Handle texture,
	uint32_t face,
	uint32_t level,
	const SourceD3D9Rect *rect,	// null for the whole level
	uint32_t front,			// volume only
	uint32_t back,			// volume only; zero for the whole depth
	int32_t readback,
	void **bits,
	int32_t *pitch,
	int32_t *slice_pitch );
void source_d3d9_texture_unlock( SourceD3D9Handle device, SourceD3D9Handle texture, uint32_t face, uint32_t level );

SourceD3D9Handle source_d3d9_buffer_create( SourceD3D9Handle device, uint32_t size, uint32_t usage, int32_t is_index, int32_t index_size );
void source_d3d9_buffer_destroy( SourceD3D9Handle device, SourceD3D9Handle buffer );
void *source_d3d9_buffer_lock( SourceD3D9Handle device, SourceD3D9Handle buffer, uint32_t offset, uint32_t size, uint32_t flags );
void source_d3d9_buffer_unlock( SourceD3D9Handle device, SourceD3D9Handle buffer );

SourceD3D9Handle source_d3d9_vertex_declaration_create( SourceD3D9Handle device, const SourceD3D9VertexElement *elements, uint32_t count );
void source_d3d9_vertex_declaration_destroy( SourceD3D9Handle device, SourceD3D9Handle declaration );

// `name` is the shader's file name, which selects the handful of per-shader
// fixups Direct3D bytecode does not carry, such as which samplers are shadow
// maps. Returns zero when the bytecode cannot be translated.
SourceD3D9Handle source_d3d9_shader_create( SourceD3D9Handle device, int32_t stage, const uint32_t *bytecode, const char *name );
void source_d3d9_shader_destroy( SourceD3D9Handle device, SourceD3D9Handle shader );

// `primitive_type` is a D3DPRIMITIVETYPE.
void source_d3d9_draw(
	SourceD3D9Handle device,
	const SourceD3D9State *state,
	uint32_t primitive_type,
	uint32_t start_vertex,
	uint32_t primitive_count );
void source_d3d9_draw_indexed(
	SourceD3D9Handle device,
	const SourceD3D9State *state,
	uint32_t primitive_type,
	int32_t base_vertex,
	uint32_t min_index,
	uint32_t vertex_count,
	uint32_t start_index,
	uint32_t primitive_count );

// `flags` is the D3DCLEAR_* mask. With no rectangles the viewport is cleared.
void source_d3d9_clear(
	SourceD3D9Handle device,
	const SourceD3D9State *state,
	uint32_t rect_count,
	const SourceD3D9Rect *rects,
	uint32_t flags,
	uint32_t color,
	float z,
	uint32_t stencil );

// `filter` is a D3DTEXTUREFILTERTYPE. Null rectangles mean the whole surface.
void source_d3d9_stretch_rect(
	SourceD3D9Handle device,
	const SourceD3D9SurfaceRef *source,
	const SourceD3D9Rect *source_rect,
	const SourceD3D9SurfaceRef *destination,
	const SourceD3D9Rect *destination_rect,
	uint32_t filter );

// Copies a render target into a system-memory surface and waits for it, which
// is how screenshots and save thumbnails read the frame back.
void source_d3d9_read_render_target(
	SourceD3D9Handle device,
	const SourceD3D9SurfaceRef *source,
	const SourceD3D9SurfaceRef *destination );

void source_d3d9_present( SourceD3D9Handle device, const SourceD3D9SurfaceRef *back_buffer );

// Occlusion queries. `issue` takes D3DISSUE_BEGIN or D3DISSUE_END. `get_data`
// returns 1 with the pixel count once the GPU has answered, 0 while it has
// not, and waits for the answer when `flush` is nonzero.
SourceD3D9Handle source_d3d9_query_create( SourceD3D9Handle device, uint32_t type );
void source_d3d9_query_destroy( SourceD3D9Handle device, SourceD3D9Handle query );
void source_d3d9_query_issue( SourceD3D9Handle device, SourceD3D9Handle query, uint32_t flags );
int32_t source_d3d9_query_get_data( SourceD3D9Handle device, SourceD3D9Handle query, int32_t flush, uint32_t *pixels );

#ifdef __cplusplus
}
#endif

#endif // SOURCE_D3D9_H
