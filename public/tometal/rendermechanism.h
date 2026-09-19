//========= Direct3D 9 on Metal ============//
//
// The render mechanism the shader API compiles against when it is built as
// shaderapimetal. It supplies the same Direct3D 9 surface ToGL does, so
// DX_TO_GL_ABSTRACTION stays defined and every POSIX code path the shader API
// already has keeps applying, but nothing here reaches OpenGL: the classes in
// tometal/dxabstract.h forward to the Rust Metal device.
//
// See docs/rust-port/d3d9-metal.md.

#ifndef TOMETAL_RENDERMECHANISM_H
#define TOMETAL_RENDERMECHANISM_H

#undef PROTECTED_THINGS_ENABLE

#include "tier0/basetypes.h"
#include "tier0/platform.h"
#include "tier0/dbg.h"
#include "bitmap/imageformat.h"

// dxabstract_types.h is shared with ToGL and describes one vertex attribute in
// OpenGL's terms. Nothing on this path reads that description, so the handful
// of names it needs are declared here rather than by pulling in OpenGL.
#ifndef __gl_h_
typedef unsigned int GLenum;
typedef unsigned int GLuint;
typedef int GLint;
typedef int GLsizei;
typedef unsigned char GLboolean;
#define GL_BYTE				0x1400
#define GL_UNSIGNED_BYTE	0x1401
#define GL_SHORT			0x1402
#define GL_UNSIGNED_SHORT	0x1403
#define GL_INT				0x1404
#define GL_UNSIGNED_INT		0x1405
#define GL_FLOAT			0x1406
#define GL_HALF_FLOAT		0x140B
#endif

#define GLMPRINTF(args)
#define GLMPRINTSTR(args)
#define GLMPRINTTEXT(args)
#define GLMDEBUG 0
#define GL_BATCH_PERF_ANALYSIS 0
#define GL_TELEMETRY_GPU_ZONES 0

#include "togl/linuxwin/dxabstract_types.h"
#include "tometal/dxabstract.h"

#endif // TOMETAL_RENDERMECHANISM_H
