//========= Direct3D 9 on Metal ============//
//
// The Direct3D 9 classes declared in public/tometal/dxabstract.h. They keep
// the bookkeeping Direct3D promises its caller, which is reference counts,
// surface descriptions and the state block, and hand everything that touches
// the GPU to the Rust device behind the C ABI in public/rust/source_d3d9.h.
//
// See docs/rust-port/d3d9-metal.md.

#include "togl/rendermechanism.h"
#include "tier0/dbg.h"
#include "tier0/threadtools.h"
#include "tier0/icommandline.h"
#include "tier1/strtools.h"
#include "tier1/convar.h"
#include "tier1/tier1.h"
#include "materialsystem/IShader.h"
#include "appframework/ilaunchermgr.h"

#include "tier0/memdbgon.h"

static IDirect3DDevice9 *g_pD3D_Device = NULL;

extern ILauncherMgr *g_pLauncherMgr;
ILauncherMgr *g_pLauncherMgr = NULL;

static void RenderedSize( uint &width, uint &height, bool set )
{
	if ( g_pLauncherMgr )
		g_pLauncherMgr->RenderedSize( width, height, set );
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// Formats
// ------------------------------------------------------------------------------------------------------------------------------ //

static bool IsDepthFormat( D3DFORMAT fmt )
{
	return fmt == D3DFMT_D16 || fmt == D3DFMT_D24X8 || fmt == D3DFMT_D24S8;
}

// The smallest rectangle of storage a level of this format needs, which for
// the block formats is a whole number of 4x4 blocks.
static uint LevelDimension( uint nBase, uint nLevel )
{
	uint n = nBase >> nLevel;
	return n ? n : 1;
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// IDirect3DResource9 / textures
// ------------------------------------------------------------------------------------------------------------------------------ //

DWORD IDirect3DResource9::SetPriority(DWORD PriorityNew)
{
	return 0;
}

IDirect3DBaseTexture9::~IDirect3DBaseTexture9()
{
	if ( m_device )
	{
		if ( m_tex )
		{
			source_d3d9_texture_destroy( m_device->m_dev, m_tex );
			m_tex = 0;
		}
		m_device = NULL;
	}
}

D3DRESOURCETYPE IDirect3DBaseTexture9::GetType()
{
	return m_restype;
}

DWORD IDirect3DBaseTexture9::GetLevelCount()
{
	return m_levels;
}

HRESULT IDirect3DBaseTexture9::GetLevelDesc(UINT Level,D3DSURFACE_DESC *pDesc)
{
	*pDesc = m_descZero;
	pDesc->Width = LevelDimension( m_descZero.Width, Level );
	pDesc->Height = LevelDimension( m_descZero.Height, Level );
	return S_OK;
}

static uint ResolveLevelCount( uint nLevels, uint nWidth, uint nHeight, uint nDepth, DWORD Usage )
{
	if ( nLevels != 0 && !( Usage & D3DUSAGE_AUTOGENMIPMAP ) )
		return nLevels;
	if ( nLevels == 1 && ( Usage & D3DUSAGE_AUTOGENMIPMAP ) == 0 )
		return 1;

	// Zero asks for the whole chain.
	uint nLargest = MAX( nWidth, MAX( nHeight, nDepth ) );
	uint nCount = 1;
	while ( nLargest > 1 )
	{
		nLargest >>= 1;
		++nCount;
	}
	return nCount;
}

static IDirect3DSurface9 *NewSurfaceView( IDirect3DDevice9 *pDevice, const D3DSURFACE_DESC &desc, SourceD3D9Handle tex, int nFace, int nMip )
{
	IDirect3DSurface9 *pSurf = new IDirect3DSurface9;
	pSurf->m_restype = (D3DRESOURCETYPE)0;	// a view of a texture, which it does not own
	pSurf->m_device = pDevice;
	pSurf->m_desc = desc;
	pSurf->m_desc.Width = LevelDimension( desc.Width, nMip );
	pSurf->m_desc.Height = LevelDimension( desc.Height, nMip );
	pSurf->m_tex = tex;
	pSurf->m_face = nFace;
	pSurf->m_mip = nMip;
	return pSurf;
}

HRESULT IDirect3DDevice9::CreateTexture(UINT Width,UINT Height,UINT Levels,DWORD Usage,D3DFORMAT Format,D3DPOOL Pool,IDirect3DTexture9** ppTexture,VD3DHANDLE* pSharedHandle, char *pDebugLabel )
{
	const uint nLevels = ResolveLevelCount( Levels, Width, Height, 1, Usage );
	SourceD3D9Handle tex = source_d3d9_texture_create( m_dev, SOURCE_D3D9_TEXTURE_2D, Width, Height, 1, nLevels, Usage, Format, pDebugLabel );
	if ( !tex )
	{
		Warning( "tometal: CreateTexture %ux%u format %u usage %08x failed\n", Width, Height, (uint)Format, (uint)Usage );
		*ppTexture = NULL;
		return D3DERR_INVALIDCALL;
	}

	IDirect3DTexture9 *dxtex = new IDirect3DTexture9;
	dxtex->m_restype = D3DRTYPE_TEXTURE;
	dxtex->m_device = this;
	dxtex->m_tex = tex;
	dxtex->m_levels = nLevels;
	dxtex->m_depth = 1;

	dxtex->m_descZero.Format = Format;
	dxtex->m_descZero.Type = D3DRTYPE_TEXTURE;
	dxtex->m_descZero.Usage = Usage;
	dxtex->m_descZero.Pool = Pool;
	dxtex->m_descZero.MultiSampleType = D3DMULTISAMPLE_NONE;
	dxtex->m_descZero.MultiSampleQuality = 0;
	dxtex->m_descZero.Width = Width;
	dxtex->m_descZero.Height = Height;

	dxtex->m_surfZero = NewSurfaceView( this, dxtex->m_descZero, tex, 0, 0 );

	*ppTexture = dxtex;
	return S_OK;
}

IDirect3DTexture9::~IDirect3DTexture9()
{
	if ( m_device )
	{
		m_device->ReleasedTexture( this );

		if ( m_surfZero )
		{
			m_surfZero->Release( 0, "~IDirect3DTexture9 public release (surfZero)" );
			m_surfZero = NULL;
		}
	}
}

HRESULT IDirect3DTexture9::LockRect(UINT Level,D3DLOCKED_RECT* pLockedRect,CONST RECT* pRect,DWORD Flags)
{
	DXABSTRACT_BREAK_ON_ERROR();
	return S_OK;
}

HRESULT IDirect3DTexture9::UnlockRect(UINT Level)
{
	DXABSTRACT_BREAK_ON_ERROR();
	return S_OK;
}

HRESULT IDirect3DTexture9::GetSurfaceLevel(UINT Level,IDirect3DSurface9** ppSurfaceLevel)
{
	// The caller is on the hook to release what this hands back.
	*ppSurfaceLevel = NewSurfaceView( m_device, m_descZero, m_tex, 0, Level );
	return S_OK;
}

HRESULT IDirect3DDevice9::CreateCubeTexture(UINT EdgeLength,UINT Levels,DWORD Usage,D3DFORMAT Format,D3DPOOL Pool,IDirect3DCubeTexture9** ppCubeTexture,VD3DHANDLE* pSharedHandle, char *pDebugLabel)
{
	const uint nLevels = ResolveLevelCount( Levels, EdgeLength, EdgeLength, 1, Usage );
	SourceD3D9Handle tex = source_d3d9_texture_create( m_dev, SOURCE_D3D9_TEXTURE_CUBE, EdgeLength, EdgeLength, 1, nLevels, Usage, Format, pDebugLabel );
	if ( !tex )
	{
		Warning( "tometal: CreateCubeTexture %u format %u usage %08x failed\n", EdgeLength, (uint)Format, (uint)Usage );
		*ppCubeTexture = NULL;
		return D3DERR_INVALIDCALL;
	}

	IDirect3DCubeTexture9 *dxtex = new IDirect3DCubeTexture9;
	dxtex->m_restype = D3DRTYPE_CUBETEXTURE;
	dxtex->m_device = this;
	dxtex->m_tex = tex;
	dxtex->m_levels = nLevels;
	dxtex->m_depth = 1;

	dxtex->m_descZero.Format = Format;
	dxtex->m_descZero.Type = D3DRTYPE_CUBETEXTURE;
	dxtex->m_descZero.Usage = Usage;
	dxtex->m_descZero.Pool = Pool;
	dxtex->m_descZero.MultiSampleType = D3DMULTISAMPLE_NONE;
	dxtex->m_descZero.MultiSampleQuality = 0;
	dxtex->m_descZero.Width = EdgeLength;
	dxtex->m_descZero.Height = EdgeLength;

	for ( int nFace = 0; nFace < 6; ++nFace )
		dxtex->m_surfZero[nFace] = NewSurfaceView( this, dxtex->m_descZero, tex, nFace, 0 );

	*ppCubeTexture = dxtex;
	return S_OK;
}

IDirect3DCubeTexture9::~IDirect3DCubeTexture9()
{
	if ( m_device )
	{
		m_device->ReleasedTexture( this );

		for ( int nFace = 0; nFace < 6; ++nFace )
		{
			if ( m_surfZero[nFace] )
			{
				m_surfZero[nFace]->Release( 0, "~IDirect3DCubeTexture9 public release (surfZero)" );
				m_surfZero[nFace] = NULL;
			}
		}
	}
}

HRESULT IDirect3DCubeTexture9::GetCubeMapSurface(D3DCUBEMAP_FACES FaceType,UINT Level,IDirect3DSurface9** ppCubeMapSurface)
{
	*ppCubeMapSurface = NewSurfaceView( m_device, m_descZero, m_tex, (int)FaceType, Level );
	return S_OK;
}

HRESULT IDirect3DCubeTexture9::GetLevelDesc(UINT Level,D3DSURFACE_DESC *pDesc)
{
	return IDirect3DBaseTexture9::GetLevelDesc( Level, pDesc );
}

HRESULT IDirect3DDevice9::CreateVolumeTexture(UINT Width,UINT Height,UINT Depth,UINT Levels,DWORD Usage,D3DFORMAT Format,D3DPOOL Pool,IDirect3DVolumeTexture9** ppVolumeTexture,VD3DHANDLE* pSharedHandle, char *pDebugLabel)
{
	const uint nLevels = ResolveLevelCount( Levels, Width, Height, Depth, Usage );
	SourceD3D9Handle tex = source_d3d9_texture_create( m_dev, SOURCE_D3D9_TEXTURE_VOLUME, Width, Height, Depth, nLevels, Usage, Format, pDebugLabel );
	if ( !tex )
	{
		Warning( "tometal: CreateVolumeTexture %ux%ux%u format %u usage %08x failed\n", Width, Height, Depth, (uint)Format, (uint)Usage );
		*ppVolumeTexture = NULL;
		return D3DERR_INVALIDCALL;
	}

	IDirect3DVolumeTexture9 *dxtex = new IDirect3DVolumeTexture9;
	dxtex->m_restype = D3DRTYPE_VOLUMETEXTURE;
	dxtex->m_device = this;
	dxtex->m_tex = tex;
	dxtex->m_levels = nLevels;
	dxtex->m_depth = Depth;

	dxtex->m_descZero.Format = Format;
	dxtex->m_descZero.Type = D3DRTYPE_VOLUMETEXTURE;
	dxtex->m_descZero.Usage = Usage;
	dxtex->m_descZero.Pool = Pool;
	dxtex->m_descZero.MultiSampleType = D3DMULTISAMPLE_NONE;
	dxtex->m_descZero.MultiSampleQuality = 0;
	dxtex->m_descZero.Width = Width;
	dxtex->m_descZero.Height = Height;

	dxtex->m_volDescZero.Format = Format;
	dxtex->m_volDescZero.Type = D3DRTYPE_VOLUMETEXTURE;
	dxtex->m_volDescZero.Usage = Usage;
	dxtex->m_volDescZero.Pool = Pool;
	dxtex->m_volDescZero.Width = Width;
	dxtex->m_volDescZero.Height = Height;
	dxtex->m_volDescZero.Depth = Depth;

	dxtex->m_surfZero = NewSurfaceView( this, dxtex->m_descZero, tex, 0, 0 );

	*ppVolumeTexture = dxtex;
	return S_OK;
}

IDirect3DVolumeTexture9::~IDirect3DVolumeTexture9()
{
	if ( m_device )
	{
		m_device->ReleasedTexture( this );

		if ( m_surfZero )
		{
			m_surfZero->Release( 0, "~IDirect3DVolumeTexture9 public release (surfZero)" );
			m_surfZero = NULL;
		}
	}
}

HRESULT IDirect3DVolumeTexture9::LockBox(UINT Level,D3DLOCKED_BOX* pLockedVolume,CONST D3DBOX* pBox,DWORD Flags)
{
	SourceD3D9Rect rect;
	const SourceD3D9Rect *pRect = NULL;
	uint nFront = 0;
	uint nBack = 0;
	if ( pBox )
	{
		rect.left = pBox->Left;
		rect.top = pBox->Top;
		rect.right = pBox->Right;
		rect.bottom = pBox->Bottom;
		pRect = &rect;
		nFront = pBox->Front;
		nBack = pBox->Back;
	}

	void *pBits = NULL;
	int nPitch = 0;
	int nSlicePitch = 0;
	const bool bReadback = ( Flags & D3DLOCK_READONLY ) != 0;
	if ( !source_d3d9_texture_lock( m_device->m_dev, m_tex, 0, Level, pRect, nFront, nBack, bReadback, &pBits, &nPitch, &nSlicePitch ) )
		return D3DERR_INVALIDCALL;

	pLockedVolume->pBits = pBits;
	pLockedVolume->RowPitch = nPitch;
	pLockedVolume->SlicePitch = nSlicePitch;
	return S_OK;
}

HRESULT IDirect3DVolumeTexture9::UnlockBox(UINT Level)
{
	source_d3d9_texture_unlock( m_device->m_dev, m_tex, 0, Level );
	return S_OK;
}

HRESULT IDirect3DVolumeTexture9::GetLevelDesc( UINT Level, D3DVOLUME_DESC *pDesc )
{
	*pDesc = m_volDescZero;
	pDesc->Width = LevelDimension( m_volDescZero.Width, Level );
	pDesc->Height = LevelDimension( m_volDescZero.Height, Level );
	pDesc->Depth = LevelDimension( m_volDescZero.Depth, Level );
	return S_OK;
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// IDirect3DSurface9
// ------------------------------------------------------------------------------------------------------------------------------ //

IDirect3DSurface9::~IDirect3DSurface9()
{
	if ( m_device )
	{
		m_device->ReleasedSurface( this );

		if ( m_restype != 0 && m_tex )
		{
			// A surface that was created on its own owns the texture under it.
			source_d3d9_texture_destroy( m_device->m_dev, m_tex );
		}
		m_tex = 0;
		m_device = NULL;
	}
}

HRESULT IDirect3DSurface9::LockRect(D3DLOCKED_RECT* pLockedRect,CONST RECT* pRect,DWORD Flags)
{
	SourceD3D9Rect rect;
	const SourceD3D9Rect *pLockRect = NULL;
	if ( pRect )
	{
		rect.left = pRect->left;
		rect.top = pRect->top;
		rect.right = pRect->right;
		rect.bottom = pRect->bottom;
		pLockRect = &rect;
	}

	void *pBits = NULL;
	int nPitch = 0;
	int nSlicePitch = 0;
	const bool bReadback = ( Flags & D3DLOCK_READONLY ) != 0;
	if ( !source_d3d9_texture_lock( m_device->m_dev, m_tex, m_face, m_mip, pLockRect, 0, 0, bReadback, &pBits, &nPitch, &nSlicePitch ) )
	{
		pLockedRect->pBits = NULL;
		pLockedRect->Pitch = 0;
		return D3DERR_INVALIDCALL;
	}

	pLockedRect->Pitch = nPitch;
	pLockedRect->pBits = pBits;
	return S_OK;
}

HRESULT IDirect3DSurface9::UnlockRect()
{
	source_d3d9_texture_unlock( m_device->m_dev, m_tex, m_face, m_mip );
	return S_OK;
}

HRESULT IDirect3DSurface9::GetDesc(D3DSURFACE_DESC *pDesc)
{
	*pDesc = m_desc;
	return S_OK;
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// IDirect3D9
// ------------------------------------------------------------------------------------------------------------------------------ //

IDirect3D9::~IDirect3D9()
{
}

UINT IDirect3D9::GetAdapterCount()
{
	return 1;
}

static void FillD3DCaps9( D3DCAPS9* pCaps )
{
	Q_memset( pCaps, 0, sizeof(*pCaps) );

	pCaps->DeviceType					=	D3DDEVTYPE_HAL;
	pCaps->Caps2						=	D3DCAPS2_DYNAMICTEXTURES;
	pCaps->DevCaps						=	D3DDEVCAPS_HWTRANSFORMANDLIGHT;
	pCaps->TextureCaps					=	D3DPTEXTURECAPS_CUBEMAP | D3DPTEXTURECAPS_MIPCUBEMAP | D3DPTEXTURECAPS_NONPOW2CONDITIONAL | D3DPTEXTURECAPS_PROJECTED;
	pCaps->PrimitiveMiscCaps			=	0;
	pCaps->RasterCaps					=	D3DPRASTERCAPS_SCISSORTEST
		|	D3DPRASTERCAPS_SLOPESCALEDEPTHBIAS
		|	D3DPRASTERCAPS_DEPTHBIAS;
	pCaps->TextureFilterCaps			=	D3DPTFILTERCAPS_MINFANISOTROPIC | D3DPTFILTERCAPS_MAGFANISOTROPIC;

	pCaps->MaxTextureWidth				=	8192;
	pCaps->MaxTextureHeight				=	8192;
	pCaps->MaxVolumeExtent				=	1024;
	pCaps->MaxTextureAspectRatio		=	0;
	pCaps->MaxAnisotropy				=	16;

	pCaps->TextureOpCaps				=	D3DTEXOPCAPS_ADD | D3DTEXOPCAPS_MODULATE2X;
	pCaps->VertexProcessingCaps			=	D3DVTXPCAPS_TEXGEN_SPHEREMAP;
	pCaps->MaxActiveLights				=	8;

	// One plane, which is what the vertex epilogue writes a clip distance for.
	pCaps->MaxUserClipPlanes			=	CommandLine()->CheckParm( "-nouserclip" ) ? 0 : 1;

	pCaps->MaxVertexBlendMatrices		=	0;
	pCaps->MaxVertexBlendMatrixIndex	=	0;
	pCaps->MaxPrimitiveCount			=	32768;
	pCaps->MaxStreams					=	D3D_MAX_STREAMS;

	pCaps->VertexShaderVersion			=	0x300;
	pCaps->MaxVertexShaderConst			=	DXABSTRACT_VS_PARAM_SLOTS;
	pCaps->PixelShaderVersion			=	0x300;

	pCaps->DevCaps2						=	D3DDEVCAPS2_STREAMOFFSET;
	pCaps->PS20Caps.NumInstructionSlots	=	512;

	pCaps->NumSimultaneousRTs					=	1;
	pCaps->MaxVertexShader30InstructionSlots	=	0;
	pCaps->MaxPixelShader30InstructionSlots		=	0;

	pCaps->FakeSRGBWrite			=	false;
	pCaps->CanDoSRGBReadFromRTs		=	true;
	pCaps->MixedSizeTargets			=	true;
}

HRESULT IDirect3D9::GetDeviceCaps(UINT Adapter, D3DDEVTYPE DeviceType, D3DCAPS9* pCaps)
{
	FillD3DCaps9( pCaps );
	return S_OK;
}

HRESULT IDirect3D9::GetAdapterIdentifier( UINT Adapter, DWORD Flags, D3DADAPTER_IDENTIFIER9* pIdentifier )
{
	Q_memset( pIdentifier, 0, sizeof(*pIdentifier) );

	SourceD3D9DisplayInfo info;
	Q_memset( &info, 0, sizeof( info ) );
	source_d3d9_display_info( &info );

	Q_snprintf( pIdentifier->Driver, sizeof(pIdentifier->Driver), "Metal %s", info.name );
	Q_snprintf( pIdentifier->Description, sizeof(pIdentifier->Description), "%s - %ux%u - %uMB",
		info.name, info.pixel_width, info.pixel_height, (uint)( info.recommended_memory >> 20 ) );

	// The shader API keys a few workarounds off the vendor. Apple's GPUs match
	// none of them, which is the point of reporting Apple's own id.
	pIdentifier->VendorId				= 0x106B;
	pIdentifier->DeviceId				= 0x0001;
	pIdentifier->SubSysId				= 0;
	pIdentifier->Revision				= 0;
	pIdentifier->VideoMemory			= (uint)MIN( info.recommended_memory, (uint64_t)0x7FFFFFFF );
	return S_OK;
}

HRESULT IDirect3D9::CheckDeviceFormat(UINT Adapter,D3DDEVTYPE DeviceType,D3DFORMAT AdapterFormat,DWORD Usage,D3DRESOURCETYPE RType,D3DFORMAT CheckFormat)
{
	if ( AdapterFormat != D3DFMT_X8R8G8B8 )
		return D3DERR_NOTAVAILABLE;

	if ( RType == D3DRTYPE_SURFACE )
		return IsDepthFormat( CheckFormat ) ? S_OK : D3DERR_NOTAVAILABLE;

	if ( RType != D3DRTYPE_TEXTURE && RType != D3DRTYPE_CUBETEXTURE && RType != D3DRTYPE_VOLUMETEXTURE )
		return D3DERR_NOTAVAILABLE;

	// What each format the Metal device creates can be used for. Anything not
	// listed is refused, and the shader API falls back to one that is: every
	// small format it knows ends its search at A8R8G8B8.
	const DWORD kSample = D3DUSAGE_DYNAMIC | D3DUSAGE_AUTOGENMIPMAP | D3DUSAGE_QUERY_FILTER;
	const DWORD kTarget = D3DUSAGE_RENDERTARGET | D3DUSAGE_QUERY_POSTPIXELSHADER_BLENDING;
	const DWORD kSRGB = D3DUSAGE_QUERY_SRGBREAD | D3DUSAGE_QUERY_SRGBWRITE;

	DWORD legalUsage = 0;
	switch ( (uint)CheckFormat )
	{
		case D3DFMT_DXT1:
		case D3DFMT_DXT3:
		case D3DFMT_DXT5:
			legalUsage = kSample | D3DUSAGE_QUERY_SRGBREAD;
			break;

		case D3DFMT_A8R8G8B8:
		case D3DFMT_X8R8G8B8:
			legalUsage = kSample | kTarget | kSRGB;
			break;

		// Widened to BGRA on upload, so sampled like it but never a target.
		case D3DFMT_R8G8B8:
		case D3DFMT_L8:
		case D3DFMT_A8L8:
			legalUsage = kSample | D3DUSAGE_QUERY_SRGBREAD;
			break;

		case D3DFMT_A16B16G16R16F:
		case D3DFMT_A16B16G16R16:
		case D3DFMT_A32B32G32R32F:
		case D3DFMT_R32F:
			legalUsage = kSample | kTarget | kSRGB;
			break;

		case D3DFMT_A8:
		case D3DFMT_V8U8:
		case D3DFMT_Q8W8V8U8:
			legalUsage = kSample;
			break;

		case D3DFMT_D16:
		case D3DFMT_D24S8:
		case D3DFMT_D24X8:
			legalUsage = D3DUSAGE_DYNAMIC | D3DUSAGE_RENDERTARGET | D3DUSAGE_DEPTHSTENCIL | D3DUSAGE_QUERY_FILTER;
			break;

		default:
			legalUsage = 0;
			return D3DERR_NOTAVAILABLE;
	}

	return ( ( Usage & legalUsage ) == Usage ) ? S_OK : D3DERR_NOTAVAILABLE;
}

// The modes offered are rendered sizes in pixels. The window manager always
// takes the desktop's own mode when it goes full screen, so none of these
// changes what the display is doing: they choose how many pixels the back
// buffer has, and the display's own pixel count is the one that is native.
static CUtlVector<D3DDISPLAYMODE> &DisplayModes()
{
	static CUtlVector<D3DDISPLAYMODE> s_Modes;
	if ( s_Modes.Count() )
		return s_Modes;

	SourceD3D9DisplayInfo info;
	Q_memset( &info, 0, sizeof( info ) );
	if ( !source_d3d9_display_info( &info ) || !info.pixel_width || !info.pixel_height )
	{
		info.pixel_width = 1920;
		info.pixel_height = 1080;
		info.refresh_hz = 60;
	}
	const uint nRefresh = info.refresh_hz ? info.refresh_hz : 60;

	static const uint s_Sizes[][2] =
	{
		{ 640, 480 }, { 800, 600 }, { 1024, 768 }, { 1152, 864 }, { 1280, 720 }, { 1280, 800 },
		{ 1280, 960 }, { 1280, 1024 }, { 1440, 900 }, { 1600, 900 }, { 1600, 1200 }, { 1680, 1050 },
		{ 1920, 1080 }, { 1920, 1200 }, { 2560, 1440 }, { 2560, 1600 }, { 3840, 2160 },
	};

	D3DDISPLAYMODE mode;
	mode.RefreshRate = nRefresh;
	mode.Format = D3DFMT_X8R8G8B8;
	for ( size_t i = 0; i < ARRAYSIZE( s_Sizes ); ++i )
	{
		// These are offscreen render sizes, not physical display modes. Metal
		// scales the back buffer to the drawable, including supersampled 4K.
		mode.Width = s_Sizes[i][0];
		mode.Height = s_Sizes[i][1];
		s_Modes.AddToTail( mode );
	}

	// The display's size in points, and then in pixels, last so that it is
	// what a caller asking for the largest mode gets.
	if ( info.backing_scale > 1.0f )
	{
		mode.Width = (uint)( info.pixel_width / info.backing_scale + 0.5f );
		mode.Height = (uint)( info.pixel_height / info.backing_scale + 0.5f );
		s_Modes.AddToTail( mode );
	}
	mode.Width = info.pixel_width;
	mode.Height = info.pixel_height;
	s_Modes.AddToTail( mode );
	return s_Modes;
}

UINT IDirect3D9::GetAdapterModeCount(UINT Adapter,D3DFORMAT Format)
{
	return DisplayModes().Count();
}

HRESULT IDirect3D9::EnumAdapterModes(UINT Adapter,D3DFORMAT Format,UINT Mode,D3DDISPLAYMODE* pMode)
{
	CUtlVector<D3DDISPLAYMODE> &modes = DisplayModes();
	if ( (int)Mode >= modes.Count() )
		return D3DERR_INVALIDCALL;
	*pMode = modes[Mode];
	return S_OK;
}

HRESULT IDirect3D9::CheckDeviceType(UINT Adapter,D3DDEVTYPE DevType,D3DFORMAT AdapterFormat,D3DFORMAT BackBufferFormat,BOOL bWindowed)
{
	return S_OK;
}

HRESULT IDirect3D9::GetAdapterDisplayMode(UINT Adapter,D3DDISPLAYMODE* pMode)
{
	CUtlVector<D3DDISPLAYMODE> &modes = DisplayModes();
	*pMode = modes[modes.Count() - 1];
	return S_OK;
}

HRESULT IDirect3D9::CheckDepthStencilMatch(UINT Adapter,D3DDEVTYPE DeviceType,D3DFORMAT AdapterFormat,D3DFORMAT RenderTargetFormat,D3DFORMAT DepthStencilFormat)
{
	return IsDepthFormat( DepthStencilFormat ) ? S_OK : D3DERR_NOTAVAILABLE;
}

HRESULT IDirect3D9::CheckDeviceMultiSampleType( UINT Adapter,D3DDEVTYPE DeviceType,D3DFORMAT SurfaceFormat,BOOL Windowed,D3DMULTISAMPLE_TYPE MultiSampleType,DWORD* pQualityLevels )
{
	// No multisampled targets yet, so only "none" is on offer.
	if ( pQualityLevels )
		*pQualityLevels = 1;
	return ( MultiSampleType == D3DMULTISAMPLE_NONE ) ? S_OK : D3DERR_NOTAVAILABLE;
}

HRESULT IDirect3D9::CreateDevice(UINT Adapter,D3DDEVTYPE DeviceType,VD3DHWND hFocusWindow,DWORD BehaviorFlags,D3DPRESENT_PARAMETERS* pPresentationParameters,IDirect3DDevice9** ppReturnedDeviceInterface)
{
	IDirect3DDevice9Params params;
	params.m_adapter = Adapter;
	params.m_deviceType = DeviceType;
	params.m_focusWindow = hFocusWindow;
	params.m_behaviorFlags = BehaviorFlags;
	params.m_presentationParameters = *pPresentationParameters;

	IDirect3DDevice9 *dev = new IDirect3DDevice9;
	HRESULT result = dev->Create( &params );
	if ( result == S_OK )
	{
		*ppReturnedDeviceInterface = dev;
	}
	else
	{
		delete dev;
		*ppReturnedDeviceInterface = NULL;
	}
	return result;
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// IDirect3DQuery9
// ------------------------------------------------------------------------------------------------------------------------------ //

HRESULT IDirect3DQuery9::Issue(DWORD dwIssueFlags)
{
	if ( m_type == D3DQUERYTYPE_OCCLUSION )
		source_d3d9_query_issue( m_device->m_dev, m_query, dwIssueFlags );
	return S_OK;
}

HRESULT IDirect3DQuery9::GetData(void* pData,DWORD dwSize,DWORD dwGetDataFlags)
{
	if ( m_type == D3DQUERYTYPE_OCCLUSION )
	{
		uint32_t nPixels = 0;
		if ( !source_d3d9_query_get_data( m_device->m_dev, m_query, ( dwGetDataFlags & D3DGETDATA_FLUSH ) != 0, &nPixels ) )
			return S_FALSE;
		if ( pData && dwSize >= sizeof( DWORD ) )
			*(DWORD *)pData = nPixels;
		return S_OK;
	}

	// An event query asks whether the GPU has caught up, and the engine only
	// uses one to throttle itself, which presenting already does.
	if ( pData && dwSize >= sizeof( BOOL ) )
		*(BOOL *)pData = TRUE;
	return S_OK;
}

HRESULT IDirect3DDevice9::CreateQuery(D3DQUERYTYPE Type,IDirect3DQuery9** ppQuery)
{
	if ( Type != D3DQUERYTYPE_OCCLUSION && Type != D3DQUERYTYPE_EVENT )
	{
		if ( ppQuery )
			*ppQuery = NULL;
		return D3DERR_NOTAVAILABLE;
	}

	// A null result pointer asks only whether the type is supported.
	if ( !ppQuery )
		return S_OK;

	IDirect3DQuery9 *pQuery = new IDirect3DQuery9;
	pQuery->m_restype = (D3DRESOURCETYPE)0;
	pQuery->m_device = this;
	pQuery->m_type = Type;
	pQuery->m_query = ( Type == D3DQUERYTYPE_OCCLUSION ) ? source_d3d9_query_create( m_dev, Type ) : 0;
	*ppQuery = pQuery;
	return S_OK;
}

IDirect3DQuery9::~IDirect3DQuery9()
{
	if ( m_device )
	{
		m_device->ReleasedQuery( this );
		if ( m_query )
			source_d3d9_query_destroy( m_device->m_dev, m_query );
		m_query = 0;
		m_device = NULL;
	}
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// Vertex and index buffers
// ------------------------------------------------------------------------------------------------------------------------------ //

HRESULT IDirect3DDevice9::CreateVertexBuffer(UINT Length,DWORD Usage,DWORD FVF,D3DPOOL Pool,IDirect3DVertexBuffer9** ppVertexBuffer,VD3DHANDLE* pSharedHandle)
{
	SourceD3D9Handle buffer = source_d3d9_buffer_create( m_dev, Length, Usage, 0, 0 );
	if ( !buffer )
	{
		*ppVertexBuffer = NULL;
		return D3DERR_OUTOFVIDEOMEMORY;
	}

	IDirect3DVertexBuffer9 *pBuffer = new IDirect3DVertexBuffer9;
	pBuffer->m_restype = D3DRTYPE_VERTEXBUFFER;
	pBuffer->m_device = this;
	pBuffer->m_vtxBuffer = buffer;
	pBuffer->m_vtxDesc.Format = D3DFMT_VERTEXDATA;
	pBuffer->m_vtxDesc.Type = D3DRTYPE_VERTEXBUFFER;
	pBuffer->m_vtxDesc.Usage = Usage;
	pBuffer->m_vtxDesc.Pool = Pool;
	pBuffer->m_vtxDesc.Size = Length;
	pBuffer->m_vtxDesc.FVF = FVF;
	*ppVertexBuffer = pBuffer;
	return S_OK;
}

IDirect3DVertexBuffer9::~IDirect3DVertexBuffer9()
{
	if ( m_device )
	{
		m_device->ReleasedVertexBuffer( this );
		if ( m_vtxBuffer )
			source_d3d9_buffer_destroy( m_device->m_dev, m_vtxBuffer );
		m_vtxBuffer = 0;
		m_device = NULL;
	}
}

HRESULT IDirect3DVertexBuffer9::Lock(UINT OffsetToLock,UINT SizeToLock,void** ppbData,DWORD Flags)
{
	*ppbData = source_d3d9_buffer_lock( m_device->m_dev, m_vtxBuffer, OffsetToLock, SizeToLock, Flags );
	return *ppbData ? S_OK : D3DERR_INVALIDCALL;
}

HRESULT IDirect3DVertexBuffer9::Unlock()
{
	source_d3d9_buffer_unlock( m_device->m_dev, m_vtxBuffer );
	return S_OK;
}

void IDirect3DVertexBuffer9::UnlockActualSize( uint nActualSize, const void *pActualData )
{
	// The lock handed out the buffer's own memory, so what was written is
	// already where it belongs however much of the range was used.
	source_d3d9_buffer_unlock( m_device->m_dev, m_vtxBuffer );
}

HRESULT IDirect3DDevice9::CreateIndexBuffer(UINT Length,DWORD Usage,D3DFORMAT Format,D3DPOOL Pool,IDirect3DIndexBuffer9** ppIndexBuffer,VD3DHANDLE* pSharedHandle)
{
	const int nIndexSize = ( Format == D3DFMT_INDEX32 ) ? 4 : 2;
	SourceD3D9Handle buffer = source_d3d9_buffer_create( m_dev, Length, Usage, 1, nIndexSize );
	if ( !buffer )
	{
		*ppIndexBuffer = NULL;
		return D3DERR_OUTOFVIDEOMEMORY;
	}

	IDirect3DIndexBuffer9 *pBuffer = new IDirect3DIndexBuffer9;
	pBuffer->m_restype = D3DRTYPE_INDEXBUFFER;
	pBuffer->m_device = this;
	pBuffer->m_idxBuffer = buffer;
	pBuffer->m_idxDesc.Format = Format;
	pBuffer->m_idxDesc.Type = D3DRTYPE_INDEXBUFFER;
	pBuffer->m_idxDesc.Usage = Usage;
	pBuffer->m_idxDesc.Pool = Pool;
	pBuffer->m_idxDesc.Size = Length;
	*ppIndexBuffer = pBuffer;
	return S_OK;
}

IDirect3DIndexBuffer9::~IDirect3DIndexBuffer9()
{
	if ( m_device )
	{
		m_device->ReleasedIndexBuffer( this );
		if ( m_idxBuffer )
			source_d3d9_buffer_destroy( m_device->m_dev, m_idxBuffer );
		m_idxBuffer = 0;
		m_device = NULL;
	}
}

HRESULT IDirect3DIndexBuffer9::Lock(UINT OffsetToLock,UINT SizeToLock,void** ppbData,DWORD Flags)
{
	*ppbData = source_d3d9_buffer_lock( m_device->m_dev, m_idxBuffer, OffsetToLock, SizeToLock, Flags );
	return *ppbData ? S_OK : D3DERR_INVALIDCALL;
}

HRESULT IDirect3DIndexBuffer9::Unlock()
{
	source_d3d9_buffer_unlock( m_device->m_dev, m_idxBuffer );
	return S_OK;
}

void IDirect3DIndexBuffer9::UnlockActualSize( uint nActualSize, const void *pActualData )
{
	source_d3d9_buffer_unlock( m_device->m_dev, m_idxBuffer );
}

HRESULT IDirect3DIndexBuffer9::GetDesc(D3DINDEXBUFFER_DESC *pDesc)
{
	*pDesc = m_idxDesc;
	return S_OK;
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// IDirect3DDevice9
// ------------------------------------------------------------------------------------------------------------------------------ //

IDirect3DDevice9::IDirect3DDevice9() :
	m_dev( 0 ),
	m_pDepthStencil( NULL ),
	m_pDefaultColorSurface( NULL ),
	m_pDefaultDepthStencilSurface( NULL ),
	m_pVertDecl( NULL ),
	m_pIndices( NULL ),
	m_vertexShader( NULL ),
	m_pixelShader( NULL ),
	m_nCurOwnerThreadId( 0 )
{
	Q_memset( &m_params, 0, sizeof( m_params ) );
	Q_memset( m_pRenderTargets, 0, sizeof( m_pRenderTargets ) );
	Q_memset( m_streamBuffers, 0, sizeof( m_streamBuffers ) );
	Q_memset( m_textures, 0, sizeof( m_textures ) );
	Q_memset( &m_state, 0, sizeof( m_state ) );
}

IDirect3DDevice9::~IDirect3DDevice9()
{
	ReleaseDefaultSurfaces();

	if ( m_dev )
	{
		source_d3d9_device_destroy( m_dev );
		m_dev = 0;
	}

	if ( g_pD3D_Device == this )
		g_pD3D_Device = NULL;
}

static uint32_t BitsOfFloat( float fl )
{
	uint32_t n;
	memcpy( &n, &fl, sizeof( n ) );
	return n;
}

// Direct3D's documented defaults, for the states a draw reads.
void IDirect3DDevice9::InitStates()
{
	uint32_t *rs = m_state.render_states;
	Q_memset( rs, 0, sizeof( m_state.render_states ) );

	rs[D3DRS_ZENABLE] = D3DZB_TRUE;
	rs[D3DRS_FILLMODE] = D3DFILL_SOLID;
	rs[D3DRS_SHADEMODE] = D3DSHADE_GOURAUD;
	rs[D3DRS_ZWRITEENABLE] = TRUE;
	rs[D3DRS_ALPHATESTENABLE] = FALSE;
	rs[D3DRS_LASTPIXEL] = TRUE;
	rs[D3DRS_SRCBLEND] = D3DBLEND_ONE;
	rs[D3DRS_DESTBLEND] = D3DBLEND_ZERO;
	rs[D3DRS_CULLMODE] = D3DCULL_CCW;
	rs[D3DRS_ZFUNC] = D3DCMP_LESSEQUAL;
	rs[D3DRS_ALPHAREF] = 0;
	rs[D3DRS_ALPHAFUNC] = D3DCMP_ALWAYS;
	rs[D3DRS_ALPHABLENDENABLE] = FALSE;
	rs[D3DRS_STENCILENABLE] = FALSE;
	rs[D3DRS_STENCILFAIL] = D3DSTENCILOP_KEEP;
	rs[D3DRS_STENCILZFAIL] = D3DSTENCILOP_KEEP;
	rs[D3DRS_STENCILPASS] = D3DSTENCILOP_KEEP;
	rs[D3DRS_STENCILFUNC] = D3DCMP_ALWAYS;
	rs[D3DRS_STENCILREF] = 0;
	rs[D3DRS_STENCILMASK] = 0xFFFFFFFF;
	rs[D3DRS_STENCILWRITEMASK] = 0xFFFFFFFF;
	rs[D3DRS_CLIPPING] = TRUE;
	rs[D3DRS_COLORWRITEENABLE] = 0x0000000F;
	rs[D3DRS_BLENDOP] = D3DBLENDOP_ADD;
	rs[D3DRS_SCISSORTESTENABLE] = FALSE;
	rs[D3DRS_SLOPESCALEDEPTHBIAS] = BitsOfFloat( 0.0f );
	rs[D3DRS_DEPTHBIAS] = BitsOfFloat( 0.0f );
	rs[D3DRS_TWOSIDEDSTENCILMODE] = FALSE;
	rs[D3DRS_CCW_STENCILFAIL] = D3DSTENCILOP_KEEP;
	rs[D3DRS_CCW_STENCILZFAIL] = D3DSTENCILOP_KEEP;
	rs[D3DRS_CCW_STENCILPASS] = D3DSTENCILOP_KEEP;
	rs[D3DRS_CCW_STENCILFUNC] = D3DCMP_ALWAYS;
	rs[D3DRS_SRGBWRITEENABLE] = FALSE;
	rs[D3DRS_SEPARATEALPHABLENDENABLE] = FALSE;
	rs[D3DRS_SRCBLENDALPHA] = D3DBLEND_ONE;
	rs[D3DRS_DESTBLENDALPHA] = D3DBLEND_ZERO;
	rs[D3DRS_BLENDOPALPHA] = D3DBLENDOP_ADD;

	for ( int nSampler = 0; nSampler < SOURCE_D3D9_SAMPLERS; ++nSampler )
	{
		uint32_t *ss = m_state.sampler_states[nSampler];
		Q_memset( ss, 0, sizeof( m_state.sampler_states[nSampler] ) );
		ss[D3DSAMP_ADDRESSU] = D3DTADDRESS_WRAP;
		ss[D3DSAMP_ADDRESSV] = D3DTADDRESS_WRAP;
		ss[D3DSAMP_ADDRESSW] = D3DTADDRESS_WRAP;
		ss[D3DSAMP_MAGFILTER] = D3DTEXF_POINT;
		ss[D3DSAMP_MINFILTER] = D3DTEXF_POINT;
		ss[D3DSAMP_MIPFILTER] = D3DTEXF_NONE;
		ss[D3DSAMP_MAXANISOTROPY] = 1;
	}

	const uint nWidth = m_params.m_presentationParameters.BackBufferWidth;
	const uint nHeight = m_params.m_presentationParameters.BackBufferHeight;
	m_state.viewport.x = 0;
	m_state.viewport.y = 0;
	m_state.viewport.width = nWidth;
	m_state.viewport.height = nHeight;
	m_state.viewport.min_z = 0.0f;
	m_state.viewport.max_z = 1.0f;
	m_state.scissor.left = 0;
	m_state.scissor.top = 0;
	m_state.scissor.right = nWidth;
	m_state.scissor.bottom = nHeight;
}

HRESULT IDirect3DDevice9::CreateDefaultSurfaces()
{
	const D3DPRESENT_PARAMETERS &pp = m_params.m_presentationParameters;

	HRESULT result = CreateRenderTarget( pp.BackBufferWidth, pp.BackBufferHeight, pp.BackBufferFormat,
		D3DMULTISAMPLE_NONE, 0, true, &m_pDefaultColorSurface, NULL, (char *)"InternalRT0" );
	if ( result != S_OK )
		return result;

	result = SetRenderTarget( 0, m_pDefaultColorSurface );
	if ( result != S_OK )
		return result;

	result = CreateDepthStencilSurface( pp.BackBufferWidth, pp.BackBufferHeight, pp.AutoDepthStencilFormat,
		D3DMULTISAMPLE_NONE, 0, TRUE, &m_pDefaultDepthStencilSurface, NULL );
	if ( result != S_OK )
		return result;

	return SetDepthStencilSurface( m_pDefaultDepthStencilSurface );
}

void IDirect3DDevice9::ReleaseDefaultSurfaces()
{
	for ( int i = 0; i < SOURCE_D3D9_RENDER_TARGETS; ++i )
		SetRenderTarget( i, NULL );
	SetDepthStencilSurface( NULL );

	if ( m_pDefaultColorSurface )
	{
		m_pDefaultColorSurface->Release( 0, "release default color surface" );
		m_pDefaultColorSurface = NULL;
	}
	if ( m_pDefaultDepthStencilSurface )
	{
		m_pDefaultDepthStencilSurface->Release( 0, "release default depthstencil surface" );
		m_pDefaultDepthStencilSurface = NULL;
	}
}

HRESULT	IDirect3DDevice9::Create( IDirect3DDevice9Params *params )
{
	g_pD3D_Device = this;
	m_params = *params;
	m_nCurOwnerThreadId = ThreadGetCurrentId();

	const D3DPRESENT_PARAMETERS &pp = m_params.m_presentationParameters;
	Msg( "tometal: creating device, back buffer %ux%u format %u\n", pp.BackBufferWidth, pp.BackBufferHeight, (uint)pp.BackBufferFormat );

	m_dev = source_d3d9_device_create( pp.BackBufferWidth, pp.BackBufferHeight );
	if ( !m_dev )
	{
		Warning( "tometal: the Metal device could not be created\n" );
		return (HRESULT)-1;
	}

	InitStates();

	HRESULT result = CreateDefaultSurfaces();
	if ( result != S_OK )
		return result;

	uint nWidth = pp.BackBufferWidth;
	uint nHeight = pp.BackBufferHeight;
	RenderedSize( nWidth, nHeight, true );
	return S_OK;
}

HRESULT IDirect3DDevice9::Reset(D3DPRESENT_PARAMETERS* pPresentationParameters)
{
	m_params.m_presentationParameters = *pPresentationParameters;
	const D3DPRESENT_PARAMETERS &pp = m_params.m_presentationParameters;
	Msg( "tometal: reset, back buffer %ux%u format %u\n", pp.BackBufferWidth, pp.BackBufferHeight, (uint)pp.BackBufferFormat );

	ReleaseDefaultSurfaces();
	source_d3d9_device_reset( m_dev, pp.BackBufferWidth, pp.BackBufferHeight );

	InitStates();

	HRESULT result = CreateDefaultSurfaces();
	if ( result != S_OK )
		return result;

	uint nWidth = pp.BackBufferWidth;
	uint nHeight = pp.BackBufferHeight;
	RenderedSize( nWidth, nHeight, true );
	return S_OK;
}

HRESULT IDirect3DDevice9::SetViewport(CONST D3DVIEWPORT9* pViewport)
{
	m_state.viewport.x = pViewport->X;
	m_state.viewport.y = pViewport->Y;
	m_state.viewport.width = pViewport->Width;
	m_state.viewport.height = pViewport->Height;
	m_state.viewport.min_z = pViewport->MinZ;
	m_state.viewport.max_z = pViewport->MaxZ;
	return S_OK;
}

HRESULT IDirect3DDevice9::GetViewport( D3DVIEWPORT9* pViewport )
{
	pViewport->X = m_state.viewport.x;
	pViewport->Y = m_state.viewport.y;
	pViewport->Width = m_state.viewport.width;
	pViewport->Height = m_state.viewport.height;
	pViewport->MinZ = m_state.viewport.min_z;
	pViewport->MaxZ = m_state.viewport.max_z;
	return S_OK;
}

HRESULT IDirect3DDevice9::BeginScene()
{
	return S_OK;
}

HRESULT IDirect3DDevice9::EndScene()
{
	return S_OK;
}

HRESULT IDirect3DDevice9::Clear(DWORD Count,CONST D3DRECT* pRects,DWORD Flags,D3DCOLOR Color,float Z,DWORD Stencil)
{
	// D3DRECT and the ABI's rectangle are both four LONG-sized ints in the
	// same order, but LONG is eight bytes here, so they are copied across.
	SourceD3D9Rect rects[8];
	uint nCount = MIN( Count, (DWORD)ARRAYSIZE( rects ) );
	if ( !pRects )
		nCount = 0;
	for ( uint i = 0; i < nCount; ++i )
	{
		rects[i].left = pRects[i].x1;
		rects[i].top = pRects[i].y1;
		rects[i].right = pRects[i].x2;
		rects[i].bottom = pRects[i].y2;
	}
	source_d3d9_clear( m_dev, &m_state, nCount, nCount ? rects : NULL, Flags, Color, Z, Stencil );
	return S_OK;
}

HRESULT IDirect3DDevice9::Present(CONST RECT* pSourceRect,CONST RECT* pDestRect,VD3DHWND hDestWindowOverride,CONST RGNDATA* pDirtyRegion)
{
	if ( m_pDefaultColorSurface )
	{
		SourceD3D9SurfaceRef backBuffer;
		backBuffer.texture = m_pDefaultColorSurface->m_tex;
		backBuffer.face = 0;
		backBuffer.level = 0;
		source_d3d9_present( m_dev, &backBuffer );
	}
	return S_OK;
}

HRESULT IDirect3DDevice9::SetTextureNonInline(DWORD Stage,IDirect3DBaseTexture9* pTexture)
{
	return SetTexture( Stage, pTexture );
}

HRESULT IDirect3DDevice9::GetTexture(DWORD Stage,IDirect3DBaseTexture9** ppTexture)
{
	if ( Stage >= SOURCE_D3D9_SAMPLERS )
	{
		*ppTexture = NULL;
		return D3DERR_INVALIDCALL;
	}
	// Direct3D adds a reference here, and ToGL never did; its caller releases
	// nothing, so neither does this.
	*ppTexture = m_textures[Stage];
	return S_OK;
}

static IDirect3DSurface9 *NewOwningSurface( IDirect3DDevice9 *pDevice, SourceD3D9Handle tex, UINT Width, UINT Height, D3DFORMAT Format, DWORD Usage, D3DPOOL Pool )
{
	IDirect3DSurface9 *pSurf = new IDirect3DSurface9;
	pSurf->m_restype = D3DRTYPE_SURFACE;	// nonzero: this surface owns its texture
	pSurf->m_device = pDevice;
	pSurf->m_desc.Format = Format;
	pSurf->m_desc.Type = D3DRTYPE_SURFACE;
	pSurf->m_desc.Usage = Usage;
	pSurf->m_desc.Pool = Pool;
	pSurf->m_desc.MultiSampleType = D3DMULTISAMPLE_NONE;
	pSurf->m_desc.MultiSampleQuality = 0;
	pSurf->m_desc.Width = Width;
	pSurf->m_desc.Height = Height;
	pSurf->m_tex = tex;
	pSurf->m_face = 0;
	pSurf->m_mip = 0;
	return pSurf;
}

HRESULT IDirect3DDevice9::CreateRenderTarget(UINT Width,UINT Height,D3DFORMAT Format,D3DMULTISAMPLE_TYPE MultiSample,DWORD MultisampleQuality,BOOL Lockable,IDirect3DSurface9** ppSurface,VD3DHANDLE* pSharedHandle, char *pDebugLabel)
{
	SourceD3D9Handle tex = source_d3d9_texture_create( m_dev, SOURCE_D3D9_TEXTURE_2D, Width, Height, 1, 1, D3DUSAGE_RENDERTARGET, Format, pDebugLabel );
	if ( !tex )
	{
		*ppSurface = NULL;
		return D3DERR_OUTOFVIDEOMEMORY;
	}
	*ppSurface = NewOwningSurface( this, tex, Width, Height, Format, D3DUSAGE_RENDERTARGET, D3DPOOL_DEFAULT );
	return S_OK;
}

HRESULT IDirect3DDevice9::SetRenderTarget(DWORD RenderTargetIndex,IDirect3DSurface9* pRenderTarget)
{
	if ( RenderTargetIndex >= SOURCE_D3D9_RENDER_TARGETS )
		return D3DERR_INVALIDCALL;

	IDirect3DSurface9 *pOld = m_pRenderTargets[RenderTargetIndex];
	if ( pRenderTarget == pOld )
		return S_OK;

	// The private count keeps a bound surface alive under a caller that has
	// already let go of it.
	if ( pRenderTarget )
		pRenderTarget->AddRef( 1, "+A  SetRenderTarget private addref" );

	m_pRenderTargets[RenderTargetIndex] = pRenderTarget;
	SourceD3D9SurfaceRef &ref = m_state.render_targets[RenderTargetIndex];
	ref.texture = pRenderTarget ? pRenderTarget->m_tex : 0;
	ref.face = pRenderTarget ? pRenderTarget->m_face : 0;
	ref.level = pRenderTarget ? pRenderTarget->m_mip : 0;

	if ( pOld )
		pOld->Release( 1, "-A  SetRenderTarget private release" );
	return S_OK;
}

HRESULT IDirect3DDevice9::GetRenderTarget(DWORD RenderTargetIndex,IDirect3DSurface9** ppRenderTarget)
{
	if ( RenderTargetIndex >= SOURCE_D3D9_RENDER_TARGETS || !ppRenderTarget )
		return D3DERR_INVALIDCALL;
	if ( !m_pRenderTargets[RenderTargetIndex] )
		return D3DERR_NOTFOUND;

	m_pRenderTargets[RenderTargetIndex]->AddRef( 0, "+B GetRenderTarget public addref" );
	*ppRenderTarget = m_pRenderTargets[RenderTargetIndex];
	return S_OK;
}

HRESULT IDirect3DDevice9::CreateOffscreenPlainSurface(UINT Width,UINT Height,D3DFORMAT Format,D3DPOOL Pool,IDirect3DSurface9** ppSurface,VD3DHANDLE* pSharedHandle)
{
	SourceD3D9Handle tex = source_d3d9_texture_create( m_dev, SOURCE_D3D9_TEXTURE_2D, Width, Height, 1, 1, 0, Format, "offscreen" );
	if ( !tex )
	{
		*ppSurface = NULL;
		return D3DERR_OUTOFVIDEOMEMORY;
	}
	*ppSurface = NewOwningSurface( this, tex, Width, Height, Format, 0, Pool );
	return S_OK;
}

HRESULT IDirect3DDevice9::CreateDepthStencilSurface(UINT Width,UINT Height,D3DFORMAT Format,D3DMULTISAMPLE_TYPE MultiSample,DWORD MultisampleQuality,BOOL Discard,IDirect3DSurface9** ppSurface,VD3DHANDLE* pSharedHandle)
{
	SourceD3D9Handle tex = source_d3d9_texture_create( m_dev, SOURCE_D3D9_TEXTURE_2D, Width, Height, 1, 1, D3DUSAGE_DEPTHSTENCIL, Format, "depthstencil" );
	if ( !tex )
	{
		*ppSurface = NULL;
		return D3DERR_OUTOFVIDEOMEMORY;
	}
	*ppSurface = NewOwningSurface( this, tex, Width, Height, Format, D3DUSAGE_DEPTHSTENCIL, D3DPOOL_DEFAULT );
	return S_OK;
}

HRESULT IDirect3DDevice9::SetDepthStencilSurface(IDirect3DSurface9* pNewZStencil)
{
	IDirect3DSurface9 *pOld = m_pDepthStencil;
	if ( pNewZStencil == pOld )
		return S_OK;

	if ( pNewZStencil )
		pNewZStencil->AddRef( 1, "+A  SetDepthStencilSurface private addref" );

	m_pDepthStencil = pNewZStencil;
	m_state.depth_stencil.texture = pNewZStencil ? pNewZStencil->m_tex : 0;
	m_state.depth_stencil.face = pNewZStencil ? pNewZStencil->m_face : 0;
	m_state.depth_stencil.level = pNewZStencil ? pNewZStencil->m_mip : 0;

	if ( pOld )
		pOld->Release( 1, "-A  SetDepthStencilSurface private release" );
	return S_OK;
}

HRESULT IDirect3DDevice9::GetDepthStencilSurface(IDirect3DSurface9** ppZStencilSurface)
{
	if ( !ppZStencilSurface )
		return D3DERR_INVALIDCALL;
	if ( !m_pDepthStencil )
	{
		*ppZStencilSurface = NULL;
		return D3DERR_NOTFOUND;
	}

	m_pDepthStencil->AddRef( 0, "+B GetDepthStencilSurface public addref" );
	*ppZStencilSurface = m_pDepthStencil;
	return S_OK;
}

static SourceD3D9SurfaceRef SurfaceRef( IDirect3DSurface9 *pSurface )
{
	SourceD3D9SurfaceRef ref;
	ref.texture = pSurface->m_tex;
	ref.face = pSurface->m_face;
	ref.level = pSurface->m_mip;
	return ref;
}

HRESULT IDirect3DDevice9::GetRenderTargetData(IDirect3DSurface9* pRenderTarget,IDirect3DSurface9* pDestSurface)
{
	if ( !pRenderTarget || !pDestSurface )
		return D3DERR_INVALIDCALL;
	SourceD3D9SurfaceRef src = SurfaceRef( pRenderTarget );
	SourceD3D9SurfaceRef dst = SurfaceRef( pDestSurface );
	source_d3d9_read_render_target( m_dev, &src, &dst );
	return S_OK;
}

HRESULT IDirect3DDevice9::GetFrontBufferData(UINT iSwapChain,IDirect3DSurface9* pDestSurface)
{
	if ( !m_pDefaultColorSurface || !pDestSurface )
		return D3DERR_INVALIDCALL;
	return GetRenderTargetData( m_pDefaultColorSurface, pDestSurface );
}

HRESULT IDirect3DDevice9::StretchRect(IDirect3DSurface9* pSourceSurface,CONST RECT* pSourceRect,IDirect3DSurface9* pDestSurface,CONST RECT* pDestRect,D3DTEXTUREFILTERTYPE Filter)
{
	if ( !pSourceSurface || !pDestSurface )
		return D3DERR_INVALIDCALL;

	SourceD3D9SurfaceRef src = SurfaceRef( pSourceSurface );
	SourceD3D9SurfaceRef dst = SurfaceRef( pDestSurface );

	SourceD3D9Rect srcRect, dstRect;
	if ( pSourceRect )
	{
		srcRect.left = pSourceRect->left;
		srcRect.top = pSourceRect->top;
		srcRect.right = pSourceRect->right;
		srcRect.bottom = pSourceRect->bottom;
	}
	if ( pDestRect )
	{
		dstRect.left = pDestRect->left;
		dstRect.top = pDestRect->top;
		dstRect.right = pDestRect->right;
		dstRect.bottom = pDestRect->bottom;
	}

	source_d3d9_stretch_rect( m_dev, &src, pSourceRect ? &srcRect : NULL, &dst, pDestRect ? &dstRect : NULL, Filter );
	return S_OK;
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// Shaders
// ------------------------------------------------------------------------------------------------------------------------------ //

HRESULT IDirect3DDevice9::CreatePixelShader(CONST DWORD* pFunction,IDirect3DPixelShader9** ppShader, const char *pShaderName, char *pDebugLabel, const uint32 *pCentroidMask )
{
	SourceD3D9Handle shader = source_d3d9_shader_create( m_dev, SOURCE_D3D9_SHADER_PIXEL, (const uint32_t *)pFunction, pShaderName );
	if ( !shader )
	{
		Warning( "tometal: pixel shader %s could not be translated\n", pShaderName ? pShaderName : "(unnamed)" );
		*ppShader = NULL;
		return D3DERR_INVALIDCALL;
	}

	IDirect3DPixelShader9 *pShader = new IDirect3DPixelShader9;
	pShader->m_restype = (D3DRESOURCETYPE)0;
	pShader->m_device = this;
	pShader->m_pixProgram = shader;
	*ppShader = pShader;
	return S_OK;
}

IDirect3DPixelShader9::~IDirect3DPixelShader9()
{
	if ( m_device )
	{
		m_device->ReleasedPixelShader( this );
		if ( m_pixProgram )
			source_d3d9_shader_destroy( m_device->m_dev, m_pixProgram );
		m_pixProgram = 0;
		m_device = NULL;
	}
}

HRESULT IDirect3DDevice9::SetPixelShaderNonInline(IDirect3DPixelShader9* pShader)
{
	return SetPixelShader( pShader );
}

HRESULT IDirect3DDevice9::SetPixelShaderConstantFNonInline(UINT StartRegister,CONST float* pConstantData,UINT Vector4fCount)
{
	return SetPixelShaderConstantF( StartRegister, pConstantData, Vector4fCount );
}

HRESULT IDirect3DDevice9::SetPixelShaderConstantB(UINT StartRegister,CONST BOOL* pConstantData,UINT BoolCount)
{
	for ( UINT i = 0; i < BoolCount && ( StartRegister + i ) < 32; ++i )
	{
		const uint32_t nBit = 1u << ( StartRegister + i );
		if ( pConstantData[i] )
			m_state.ps_bools |= nBit;
		else
			m_state.ps_bools &= ~nBit;
	}
	return S_OK;
}

HRESULT IDirect3DDevice9::SetPixelShaderConstantI(UINT StartRegister,CONST int* pConstantData,UINT Vector4iCount)
{
	if ( StartRegister < SOURCE_D3D9_INT_CONSTANTS )
	{
		UINT nCount = MIN( Vector4iCount, SOURCE_D3D9_INT_CONSTANTS - StartRegister );
		memcpy( m_state.ps_ints[StartRegister], pConstantData, nCount * 4 * sizeof( int ) );
	}
	return S_OK;
}

HRESULT IDirect3DDevice9::CreateVertexShader(CONST DWORD* pFunction,IDirect3DVertexShader9** ppShader, const char *pShaderName, char *pDebugLabel)
{
	SourceD3D9Handle shader = source_d3d9_shader_create( m_dev, SOURCE_D3D9_SHADER_VERTEX, (const uint32_t *)pFunction, pShaderName );
	if ( !shader )
	{
		Warning( "tometal: vertex shader %s could not be translated\n", pShaderName ? pShaderName : "(unnamed)" );
		*ppShader = NULL;
		return D3DERR_INVALIDCALL;
	}

	IDirect3DVertexShader9 *pShader = new IDirect3DVertexShader9;
	pShader->m_restype = (D3DRESOURCETYPE)0;
	pShader->m_device = this;
	pShader->m_vtxProgram = shader;
	*ppShader = pShader;
	return S_OK;
}

IDirect3DVertexShader9::~IDirect3DVertexShader9()
{
	if ( m_device )
	{
		m_device->ReleasedVertexShader( this );
		if ( m_vtxProgram )
			source_d3d9_shader_destroy( m_device->m_dev, m_vtxProgram );
		m_vtxProgram = 0;
		m_device = NULL;
	}
}

HRESULT IDirect3DDevice9::SetVertexShaderNonInline(IDirect3DVertexShader9* pShader)
{
	return SetVertexShader( pShader );
}

HRESULT IDirect3DDevice9::SetVertexShaderConstantFNonInline(UINT StartRegister,CONST float* pConstantData,UINT Vector4fCount)
{
	return SetVertexShaderConstantF( StartRegister, pConstantData, Vector4fCount );
}

HRESULT IDirect3DDevice9::SetVertexShaderConstantBNonInline(UINT StartRegister,CONST BOOL* pConstantData,UINT BoolCount)
{
	return SetVertexShaderConstantB( StartRegister, pConstantData, BoolCount );
}

HRESULT IDirect3DDevice9::SetVertexShaderConstantINonInline(UINT StartRegister,CONST int* pConstantData,UINT Vector4iCount)
{
	return SetVertexShaderConstantI( StartRegister, pConstantData, Vector4iCount );
}

HRESULT IDirect3DDevice9::LinkShaderPair( IDirect3DVertexShader9* vs, IDirect3DPixelShader9* ps )
{
	// A pipeline needs the vertex layout and target formats as well as the
	// pair, so there is nothing to build ahead of the first draw that uses it.
	return S_OK;
}

HRESULT IDirect3DDevice9::ValidateShaderPair( IDirect3DVertexShader9* vs, IDirect3DPixelShader9* ps )
{
	return S_OK;
}

HRESULT IDirect3DDevice9::QueryShaderPair( int index, GLMShaderPairInfo *infoOut )
{
	// Reports the shader pairs a run linked, for ToGL's on-disk cache of them.
	// There is no such cache here, and a negative status ends the caller's walk.
	infoOut->m_status = -1;
	return S_OK;
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// Vertex declarations, streams
// ------------------------------------------------------------------------------------------------------------------------------ //

HRESULT IDirect3DDevice9::CreateVertexDeclaration(CONST D3DVERTEXELEMENT9* pVertexElements,IDirect3DVertexDeclaration9** ppDecl)
{
	IDirect3DVertexDeclaration9 *pDecl = new IDirect3DVertexDeclaration9;
	pDecl->m_device = this;
	pDecl->m_elemCount = 0;

	SourceD3D9VertexElement elements[MAX_D3DVERTEXELEMENTS];
	for ( const D3DVERTEXELEMENT9 *pSrc = pVertexElements; pSrc->Stream != 0xFF && pDecl->m_elemCount < MAX_D3DVERTEXELEMENTS; ++pSrc )
	{
		pDecl->m_elements[pDecl->m_elemCount] = *pSrc;
		SourceD3D9VertexElement &dst = elements[pDecl->m_elemCount];
		dst.stream = pSrc->Stream;
		dst.offset = pSrc->Offset;
		dst.type = pSrc->Type;
		dst.method = pSrc->Method;
		dst.usage = pSrc->Usage;
		dst.usage_index = pSrc->UsageIndex;
		++pDecl->m_elemCount;
	}

	pDecl->m_decl = source_d3d9_vertex_declaration_create( m_dev, elements, pDecl->m_elemCount );
	*ppDecl = pDecl;
	return S_OK;
}

IDirect3DVertexDeclaration9::~IDirect3DVertexDeclaration9()
{
	if ( m_device )
	{
		m_device->ReleasedVertexDeclaration( this );
		if ( m_decl )
			source_d3d9_vertex_declaration_destroy( m_device->m_dev, m_decl );
		m_decl = 0;
		m_device = NULL;
	}
}

HRESULT IDirect3DDevice9::SetVertexDeclarationNonInline(IDirect3DVertexDeclaration9* pDecl)
{
	return SetVertexDeclaration( pDecl );
}

HRESULT IDirect3DDevice9::SetFVF(DWORD FVF)
{
	return S_OK;
}

HRESULT IDirect3DDevice9::GetFVF(DWORD* pFVF)
{
	if ( pFVF )
		*pFVF = 0;
	return S_OK;
}

HRESULT IDirect3DDevice9::SetStreamSourceNonInline(UINT StreamNumber,IDirect3DVertexBuffer9* pStreamData,UINT OffsetInBytes,UINT Stride)
{
	return SetStreamSource( StreamNumber, pStreamData, OffsetInBytes, Stride );
}

HRESULT IDirect3DDevice9::SetIndicesNonInline(IDirect3DIndexBuffer9* pIndexData)
{
	return SetIndices( pIndexData );
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// Retired objects. Each scrubs itself out of the state block before its
// handle dies, which is what lets a draw trust every handle it is given.
// ------------------------------------------------------------------------------------------------------------------------------ //

void IDirect3DDevice9::ReleasedVertexDeclaration( IDirect3DVertexDeclaration9 *pDecl )
{
	if ( m_pVertDecl == pDecl )
		SetVertexDeclaration( NULL );
}

void IDirect3DDevice9::ReleasedTexture( IDirect3DBaseTexture9 *baseTex )
{
	for ( int i = 0; i < SOURCE_D3D9_SAMPLERS; ++i )
	{
		if ( m_textures[i] == baseTex )
			SetTexture( i, NULL );
	}
}

void IDirect3DDevice9::ReleasedSurface( IDirect3DSurface9 *surface )
{
	for ( int i = 0; i < SOURCE_D3D9_RENDER_TARGETS; ++i )
	{
		if ( m_pRenderTargets[i] == surface )
		{
			// Reached only through the last reference going away, so there is
			// no private count left to drop.
			m_pRenderTargets[i] = NULL;
			Q_memset( &m_state.render_targets[i], 0, sizeof( m_state.render_targets[i] ) );
		}
	}

	if ( m_pDepthStencil == surface )
	{
		m_pDepthStencil = NULL;
		Q_memset( &m_state.depth_stencil, 0, sizeof( m_state.depth_stencil ) );
	}

	if ( m_pDefaultColorSurface == surface )
		m_pDefaultColorSurface = NULL;
	if ( m_pDefaultDepthStencilSurface == surface )
		m_pDefaultDepthStencilSurface = NULL;
}

void IDirect3DDevice9::ReleasedPixelShader( IDirect3DPixelShader9 *pixelShader )
{
	if ( m_pixelShader == pixelShader )
		SetPixelShader( NULL );
}

void IDirect3DDevice9::ReleasedVertexShader( IDirect3DVertexShader9 *vertexShader )
{
	if ( m_vertexShader == vertexShader )
		SetVertexShader( NULL );
}

void IDirect3DDevice9::ReleasedVertexBuffer( IDirect3DVertexBuffer9 *vertexBuffer )
{
	for ( int i = 0; i < SOURCE_D3D9_STREAMS; ++i )
	{
		if ( m_streamBuffers[i] == vertexBuffer )
			SetStreamSource( i, NULL, 0, 0 );
	}
}

void IDirect3DDevice9::ReleasedIndexBuffer( IDirect3DIndexBuffer9 *indexBuffer )
{
	if ( m_pIndices == indexBuffer )
		SetIndices( NULL );
}

void IDirect3DDevice9::ReleasedQuery( IDirect3DQuery9 *query )
{
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// State and drawing
// ------------------------------------------------------------------------------------------------------------------------------ //

HRESULT IDirect3DDevice9::SetRenderState( D3DRENDERSTATETYPE State, DWORD Value )
{
	return SetRenderStateInline( State, Value );
}

HRESULT IDirect3DDevice9::SetSamplerStateNonInline( DWORD Sampler, D3DSAMPLERSTATETYPE Type, DWORD Value )
{
	return SetSamplerState( Sampler, Type, Value );
}

void IDirect3DDevice9::SetSamplerStatesNonInline(
	DWORD Sampler, DWORD AddressU, DWORD AddressV, DWORD AddressW,
	DWORD MinFilter, DWORD MagFilter, DWORD MipFilter, DWORD MinLod, float LodBias )
{
	SetSamplerStates( Sampler, AddressU, AddressV, AddressW, MinFilter, MagFilter, MipFilter, MinLod, LodBias );
}

#ifdef OSX
HRESULT IDirect3DDevice9::FlushIndexBindings()
{
	return S_OK;
}

HRESULT IDirect3DDevice9::FlushVertexBindings( uint baseVertexIndex )
{
	return S_OK;
}
#endif

HRESULT IDirect3DDevice9::DrawPrimitive(D3DPRIMITIVETYPE PrimitiveType,UINT StartVertex,UINT PrimitiveCount)
{
	source_d3d9_draw( m_dev, &m_state, PrimitiveType, StartVertex, PrimitiveCount );
	return S_OK;
}

HRESULT IDirect3DDevice9::DrawIndexedPrimitive(D3DPRIMITIVETYPE PrimitiveType,INT BaseVertexIndex,UINT MinVertexIndex,UINT NumVertices,UINT startIndex,UINT primCount)
{
	source_d3d9_draw_indexed( m_dev, &m_state, PrimitiveType, BaseVertexIndex, MinVertexIndex, NumVertices, startIndex, primCount );
	return S_OK;
}

HRESULT IDirect3DDevice9::DrawIndexedPrimitiveUP(D3DPRIMITIVETYPE PrimitiveType,UINT MinVertexIndex,UINT NumVertices,UINT PrimitiveCount,CONST void* pIndexData,D3DFORMAT IndexDataFormat,CONST void* pVertexStreamZeroData,UINT VertexStreamZeroStride)
{
	DXABSTRACT_BREAK_ON_ERROR();
	return S_OK;
}

BOOL IDirect3DDevice9::ShowCursor(BOOL bShow)
{
	return TRUE;
}

HRESULT IDirect3DDevice9::ValidateDevice(DWORD* pNumPasses)
{
	if ( pNumPasses )
		*pNumPasses = 1;
	return S_OK;
}

HRESULT IDirect3DDevice9::SetMaterial(CONST D3DMATERIAL9* pMaterial)
{
	return S_OK;
}

HRESULT IDirect3DDevice9::LightEnable(DWORD Index,BOOL Enable)
{
	return S_OK;
}

HRESULT IDirect3DDevice9::SetScissorRect(CONST RECT* pRect)
{
	m_state.scissor.left = pRect->left;
	m_state.scissor.top = pRect->top;
	m_state.scissor.right = pRect->right;
	m_state.scissor.bottom = pRect->bottom;
	return S_OK;
}

HRESULT IDirect3DDevice9::GetDeviceCaps(D3DCAPS9* pCaps)
{
	FillD3DCaps9( pCaps );
	return S_OK;
}

HRESULT IDirect3DDevice9::TestCooperativeLevel()
{
	return S_OK;
}

HRESULT IDirect3DDevice9::EvictManagedResources()
{
	return S_OK;
}

HRESULT IDirect3DDevice9::SetLight(DWORD Index,CONST D3DLIGHT9*)
{
	return S_OK;
}

void IDirect3DDevice9::SetGammaRamp(UINT iSwapChain,DWORD Flags,CONST D3DGAMMARAMP* pRamp)
{
}

void IDirect3DDevice9::SaveGLState()
{
}

void IDirect3DDevice9::RestoreGLState()
{
}

HRESULT IDirect3DDevice9::SetClipPlane(DWORD Index,CONST float* pPlane)
{
	if ( Index == 0 )
		memcpy( m_state.clip_plane0, pPlane, sizeof( m_state.clip_plane0 ) );
	return S_OK;
}

HRESULT IDirect3DDevice9::SetTransform(D3DTRANSFORMSTATETYPE State,CONST D3DMATRIX* pMatrix)
{
	return S_OK;
}

HRESULT IDirect3DDevice9::SetTextureStageState(DWORD Stage,D3DTEXTURESTAGESTATETYPE Type,DWORD Value)
{
	return S_OK;
}

void IDirect3DDevice9::AcquireThreadOwnership()
{
	m_nCurOwnerThreadId = ThreadGetCurrentId();
}

void IDirect3DDevice9::ReleaseThreadOwnership()
{
	m_nCurOwnerThreadId = 0;
}

void IDirect3DDevice9::SetMaxUsedVertexShaderConstantsHintNonInline( uint nMaxReg )
{
}

// ------------------------------------------------------------------------------------------------------------------------------ //

IDirect3D9 *Direct3DCreate9(UINT SDKVersion)
{
	return new IDirect3D9;
}

void D3DPERF_SetOptions( DWORD dwOptions )
{
}

HRESULT D3DXCompileShader(
	LPCSTR                          pSrcData,
	UINT                            SrcDataLen,
	CONST D3DXMACRO*                pDefines,
	LPD3DXINCLUDE                   pInclude,
	LPCSTR                          pFunctionName,
	LPCSTR                          pProfile,
	DWORD                           Flags,
	LPD3DXBUFFER*                   ppShader,
	LPD3DXBUFFER*                   ppErrorMsgs,
	LPD3DXCONSTANTTABLE*            ppConstantTable)
{
	DXABSTRACT_BREAK_ON_ERROR();
	return S_OK;
}

void toglGetClientRect( VD3DHWND hWnd, RECT *destRect )
{
	// The only useful answer is the size being rendered at.
	uint width = 0;
	uint height = 0;
	RenderedSize( width, height, false );

	destRect->left = 0;
	destRect->top = 0;
	destRect->right = width;
	destRect->bottom = height;
}

// On OS X the client only tests glow and sun visibility with occlusion queries
// when this says they are cheap, which ToGL decided from the OpenGL driver's
// version. The client declares the same variable and shares this one's value.
// Metal's visibility counting costs nothing to issue and is polled, not waited
// on, so the answer is always yes.
static ConVar gl_can_query_fast( "gl_can_query_fast", "1" );

// The shader API connects its render mechanism through this, which is where
// ToGL resolved its OpenGL entry points. Here it finds the window manager and
// registers the console variables above.
void TometalConnectLibraries( CreateInterfaceFn factory )
{
	ConnectTier1Libraries( &factory, 1 );
	ConVar_Register();
	g_pLauncherMgr = (ILauncherMgr *)factory( SDLMGR_INTERFACE_VERSION, NULL );
}

// Takes the console variables back out before this library is unloaded, or
// the console is left pointing into memory that is gone.
void TometalDisconnectLibraries()
{
	ConVar_Unregister();
	DisconnectTier1Libraries();
	g_pLauncherMgr = NULL;
}
