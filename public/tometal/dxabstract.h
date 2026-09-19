//========= Direct3D 9 on Metal ============//
//
// The Direct3D 9 classes the shader API is written against, in the shape ToGL
// declares them so that shaderapidx9's sources compile unchanged, but holding
// handles into the Rust Metal device rather than OpenGL objects.
//
// The device keeps the Direct3D state machine in one plain block. Setting
// state is an array write, inlined here, and nothing crosses into Rust until
// something is drawn, cleared, copied, locked or presented.
//
// See docs/rust-port/d3d9-metal.md.

#ifndef TOMETAL_DXABSTRACT_H
#define TOMETAL_DXABSTRACT_H

#include "tier0/platform.h"
#include "tier0/dbg.h"
#include "tier0/threadtools.h"
#include "tier1/utlvector.h"
#include "tier1/interface.h"
#include "rust/source_d3d9.h"

// What ToGL reports about one linked shader pair, for its on-disk cache of
// them. Kept in ToGL's shape because the shader API walks it; this device
// reports none, so only m_status is ever filled in.
struct GLMShaderPairInfo
{
	int		m_status;		// -1 means req'd index was out of bounds (loop stop..)  0 means not present.  1 means present/active.

	char	m_vsName[ 128 ];
	int		m_vsStaticIndex;
	int		m_vsDynamicIndex;

	char	m_psName[ 128 ];
	int		m_psStaticIndex;
	int		m_psDynamicIndex;
};

TOGL_INTERFACE void toglGetClientRect( VD3DHWND hWnd, RECT *destRect );

// Where ToGL resolved its OpenGL entry points, this finds the window manager,
// which is told the size being rendered at.
TOGL_INTERFACE void TometalConnectLibraries( CreateInterfaceFn factory );
TOGL_INTERFACE void TometalDisconnectLibraries();

struct TOGL_CLASS IUnknown
{
	int	m_refcount[2];
	bool m_mark;

	IUnknown()
	{
		m_refcount[0] = 1;
		m_refcount[1] = 0;
		m_mark = false;
	};

	virtual	~IUnknown()
	{
	};

	void	AddRef( int which=0, char *comment = NULL )
	{
		Assert( which >= 0 );
		Assert( which < 2 );
		m_refcount[which]++;
	};

	ULONG __stdcall	Release( int which=0, char *comment = NULL )
	{
		Assert( which >= 0 );
		Assert( which < 2 );

		m_refcount[which]--;
		if ( (!m_refcount[0]) && (!m_refcount[1]) )
		{
			delete this;
			return 0;
		}
		return m_refcount[0];
	};

	void	SetMark( bool markValue, char *comment=NULL )
	{
		m_mark = markValue;
	}
};

// ------------------------------------------------------------------------------------------------------------------------------ //
// INTERFACES
// ------------------------------------------------------------------------------------------------------------------------------ //

struct TOGL_CLASS IDirect3DResource9 : public IUnknown
{
	IDirect3DDevice9	*m_device;		// parent device
	D3DRESOURCETYPE		m_restype;

	DWORD SetPriority(DWORD PriorityNew);
};

struct TOGL_CLASS IDirect3DBaseTexture9 : public IDirect3DResource9
{
	D3DSURFACE_DESC			m_descZero;			// desc of top level.
	SourceD3D9Handle		m_tex;				// the Metal texture, of whichever dimensionality
	uint					m_levels;
	uint					m_depth;			// volumes only

	virtual					~IDirect3DBaseTexture9();
	D3DRESOURCETYPE	TOGLMETHODCALLTYPE GetType();
	DWORD TOGLMETHODCALLTYPE GetLevelCount();
	HRESULT TOGLMETHODCALLTYPE GetLevelDesc(UINT Level,D3DSURFACE_DESC *pDesc);
};

struct TOGL_CLASS IDirect3DTexture9 : public IDirect3DBaseTexture9
{
	IDirect3DSurface9 *m_surfZero;				// surf of top level.
	virtual ~IDirect3DTexture9();
	HRESULT TOGLMETHODCALLTYPE LockRect(UINT Level,D3DLOCKED_RECT* pLockedRect,CONST RECT* pRect,DWORD Flags);
	HRESULT TOGLMETHODCALLTYPE UnlockRect(UINT Level);
	HRESULT TOGLMETHODCALLTYPE GetSurfaceLevel(UINT Level,IDirect3DSurface9** ppSurfaceLevel);
};

struct TOGL_CLASS IDirect3DCubeTexture9 : public IDirect3DBaseTexture9
{
	IDirect3DSurface9		*m_surfZero[6];			// surfs of top level.
	virtual ~IDirect3DCubeTexture9();
	HRESULT TOGLMETHODCALLTYPE GetCubeMapSurface(D3DCUBEMAP_FACES FaceType,UINT Level,IDirect3DSurface9** ppCubeMapSurface);
	HRESULT TOGLMETHODCALLTYPE GetLevelDesc(UINT Level,D3DSURFACE_DESC *pDesc);
};

struct TOGL_CLASS IDirect3DVolumeTexture9 : public IDirect3DBaseTexture9
{
	IDirect3DSurface9		*m_surfZero;			// surf of top level.
	D3DVOLUME_DESC			m_volDescZero;			// volume desc top level
	virtual ~IDirect3DVolumeTexture9();
	HRESULT TOGLMETHODCALLTYPE LockBox(UINT Level,D3DLOCKED_BOX* pLockedVolume,CONST D3DBOX* pBox,DWORD Flags);
	HRESULT TOGLMETHODCALLTYPE UnlockBox(UINT Level);
	HRESULT	TOGLMETHODCALLTYPE GetLevelDesc( UINT level, D3DVOLUME_DESC *pDesc );
};

// A surface is one face and mip of a texture. One made by CreateRenderTarget,
// CreateDepthStencilSurface or CreateOffscreenPlainSurface owns its texture,
// which m_restype being nonzero records; one handed out by a texture does not.
struct TOGL_CLASS IDirect3DSurface9 : public IDirect3DResource9
{
	virtual ~IDirect3DSurface9();
	HRESULT TOGLMETHODCALLTYPE LockRect(D3DLOCKED_RECT* pLockedRect,CONST RECT* pRect,DWORD Flags);
	HRESULT TOGLMETHODCALLTYPE UnlockRect();
	HRESULT TOGLMETHODCALLTYPE GetDesc(D3DSURFACE_DESC *pDesc);

	D3DSURFACE_DESC			m_desc;
	SourceD3D9Handle		m_tex;
	int						m_face;
	int						m_mip;
};

struct TOGL_CLASS IDirect3D9 : public IUnknown
{
	virtual	~IDirect3D9();

	UINT	TOGLMETHODCALLTYPE GetAdapterCount();

	HRESULT TOGLMETHODCALLTYPE GetDeviceCaps				(UINT Adapter,D3DDEVTYPE DeviceType,D3DCAPS9* pCaps);
	HRESULT TOGLMETHODCALLTYPE GetAdapterIdentifier			(UINT Adapter,DWORD Flags,D3DADAPTER_IDENTIFIER9* pIdentifier);
	HRESULT TOGLMETHODCALLTYPE CheckDeviceFormat			(UINT Adapter,D3DDEVTYPE DeviceType,D3DFORMAT AdapterFormat,DWORD Usage,D3DRESOURCETYPE RType,D3DFORMAT CheckFormat);
	UINT	TOGLMETHODCALLTYPE GetAdapterModeCount			(UINT Adapter,D3DFORMAT Format);
	HRESULT TOGLMETHODCALLTYPE EnumAdapterModes				(UINT Adapter,D3DFORMAT Format,UINT Mode,D3DDISPLAYMODE* pMode);
	HRESULT TOGLMETHODCALLTYPE CheckDeviceType				(UINT Adapter,D3DDEVTYPE DevType,D3DFORMAT AdapterFormat,D3DFORMAT BackBufferFormat,BOOL bWindowed);
	HRESULT TOGLMETHODCALLTYPE GetAdapterDisplayMode		(UINT Adapter,D3DDISPLAYMODE* pMode);
	HRESULT TOGLMETHODCALLTYPE CheckDepthStencilMatch		(UINT Adapter,D3DDEVTYPE DeviceType,D3DFORMAT AdapterFormat,D3DFORMAT RenderTargetFormat,D3DFORMAT DepthStencilFormat);
	HRESULT TOGLMETHODCALLTYPE CheckDeviceMultiSampleType	(UINT Adapter,D3DDEVTYPE DeviceType,D3DFORMAT SurfaceFormat,BOOL Windowed,D3DMULTISAMPLE_TYPE MultiSampleType,DWORD* pQualityLevels);

	HRESULT TOGLMETHODCALLTYPE CreateDevice					(UINT Adapter,D3DDEVTYPE DeviceType,VD3DHWND hFocusWindow,DWORD BehaviorFlags,D3DPRESENT_PARAMETERS* pPresentationParameters,IDirect3DDevice9** ppReturnedDeviceInterface);
};

struct TOGL_CLASS IDirect3DVertexDeclaration9 : public IUnknown
{
	IDirect3DDevice9		*m_device;
	uint					m_elemCount;
	D3DVERTEXELEMENT9		m_elements[ MAX_D3DVERTEXELEMENTS ];
	SourceD3D9Handle		m_decl;

	virtual					~IDirect3DVertexDeclaration9();
};

struct TOGL_CLASS IDirect3DQuery9 : public IDirect3DResource9
{
	D3DQUERYTYPE			m_type;		// D3DQUERYTYPE_OCCLUSION or D3DQUERYTYPE_EVENT
	SourceD3D9Handle		m_query;

	virtual					~IDirect3DQuery9();

	HRESULT					Issue(DWORD dwIssueFlags);
	HRESULT					GetData(void* pData,DWORD dwSize,DWORD dwGetDataFlags);
};

struct TOGL_CLASS IDirect3DVertexBuffer9 : public IDirect3DResource9
{
	SourceD3D9Handle		m_vtxBuffer;
	D3DVERTEXBUFFER_DESC	m_vtxDesc;		// to satisfy GetDesc

	virtual					~IDirect3DVertexBuffer9();
	HRESULT					Lock(UINT OffsetToLock,UINT SizeToLock,void** ppbData,DWORD Flags);
	HRESULT					Unlock();
	void					UnlockActualSize( uint nActualSize, const void *pActualData = NULL );
};

struct TOGL_CLASS IDirect3DIndexBuffer9 : public IDirect3DResource9
{
	SourceD3D9Handle		m_idxBuffer;
	D3DINDEXBUFFER_DESC		m_idxDesc;		// to satisfy GetDesc

	virtual					~IDirect3DIndexBuffer9();

	HRESULT					Lock(UINT OffsetToLock,UINT SizeToLock,void** ppbData,DWORD Flags);
	HRESULT					Unlock();
	void					UnlockActualSize( uint nActualSize, const void *pActualData = NULL );

	HRESULT					GetDesc(D3DINDEXBUFFER_DESC *pDesc);
};

struct TOGL_CLASS IDirect3DPixelShader9 : public IDirect3DResource9
{
	SourceD3D9Handle		m_pixProgram;

	virtual					~IDirect3DPixelShader9();
};

struct TOGL_CLASS IDirect3DVertexShader9 : public IDirect3DResource9
{
	SourceD3D9Handle		m_vtxProgram;

	virtual					~IDirect3DVertexShader9();
};

typedef class CUtlMemory<D3DMATRIX> CD3DMATRIXAllocator;
typedef class CUtlVector<D3DMATRIX, CD3DMATRIXAllocator> CD3DMATRIXStack;

struct TOGL_CLASS ID3DXMatrixStack //: public IUnknown
{
	int	m_refcount[2];
	bool m_mark;
	CD3DMATRIXStack	m_stack;
	int						m_stackTop;	// top of stack is at the highest index, this is that index.  push increases, pop decreases.

	ID3DXMatrixStack();
	void  AddRef( int which=0, char *comment = NULL );
	ULONG Release( int which=0, char *comment = NULL );

	HRESULT	Create( void );

	D3DXMATRIX* GetTop();
	void Push();
	void Pop();
	void LoadIdentity();
	void LoadMatrix( const D3DXMATRIX *pMat );
	void MultMatrix( const D3DXMATRIX *pMat );
	void MultMatrixLocal( const D3DXMATRIX *pMat );
	HRESULT ScaleLocal(FLOAT x, FLOAT y, FLOAT z);

	// Left multiply the current matrix with the computed rotation
	// matrix, counterclockwise about the given axis with the given angle.
	// (rotation is about the local origin of the object)
	HRESULT RotateAxisLocal(CONST D3DXVECTOR3* pV, FLOAT Angle);

	// Left multiply the current matrix with the computed translation
	// matrix. (transformation is about the local origin of the object)
	HRESULT TranslateLocal(FLOAT x, FLOAT y, FLOAT z);
};

typedef ID3DXMatrixStack* LPD3DXMATRIXSTACK;

struct TOGL_CLASS IDirect3DDevice9 : public IUnknown
{
	friend struct IDirect3DBaseTexture9;
	friend struct IDirect3DTexture9;
	friend struct IDirect3DCubeTexture9;
	friend struct IDirect3DVolumeTexture9;
	friend struct IDirect3DSurface9;
	friend struct IDirect3DVertexBuffer9;
	friend struct IDirect3DIndexBuffer9;
	friend struct IDirect3DPixelShader9;
	friend struct IDirect3DVertexShader9;
	friend struct IDirect3DQuery9;
	friend struct IDirect3DVertexDeclaration9;

	IDirect3DDevice9();
	virtual	~IDirect3DDevice9();

	// Create call invoked from IDirect3D9
	HRESULT	TOGLMETHODCALLTYPE Create( IDirect3DDevice9Params *params );

	//
	// Basics
	//
	HRESULT TOGLMETHODCALLTYPE Reset(D3DPRESENT_PARAMETERS* pPresentationParameters);
	HRESULT TOGLMETHODCALLTYPE SetViewport(CONST D3DVIEWPORT9* pViewport);
	HRESULT TOGLMETHODCALLTYPE GetViewport(D3DVIEWPORT9* pViewport);
	HRESULT TOGLMETHODCALLTYPE BeginScene();
	HRESULT TOGLMETHODCALLTYPE Clear(DWORD Count,CONST D3DRECT* pRects,DWORD Flags,D3DCOLOR Color,float Z,DWORD Stencil);
	HRESULT TOGLMETHODCALLTYPE EndScene();
	HRESULT TOGLMETHODCALLTYPE Present(CONST RECT* pSourceRect,CONST RECT* pDestRect,VD3DHWND hDestWindowOverride,CONST RGNDATA* pDirtyRegion);

	// textures
	HRESULT TOGLMETHODCALLTYPE CreateTexture(UINT Width,UINT Height,UINT Levels,DWORD Usage,D3DFORMAT Format,D3DPOOL Pool,IDirect3DTexture9** ppTexture,VD3DHANDLE* pSharedHandle, char *debugLabel=NULL);
	HRESULT TOGLMETHODCALLTYPE CreateCubeTexture(UINT EdgeLength,UINT Levels,DWORD Usage,D3DFORMAT Format,D3DPOOL Pool,IDirect3DCubeTexture9** ppCubeTexture,VD3DHANDLE* pSharedHandle, char *debugLabel=NULL);
	HRESULT TOGLMETHODCALLTYPE CreateVolumeTexture(UINT Width,UINT Height,UINT Depth,UINT Levels,DWORD Usage,D3DFORMAT Format,D3DPOOL Pool,IDirect3DVolumeTexture9** ppVolumeTexture,VD3DHANDLE* pSharedHandle, char *debugLabel=NULL);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetTexture(DWORD Stage,IDirect3DBaseTexture9* pTexture);
	HRESULT TOGLMETHODCALLTYPE SetTextureNonInline(DWORD Stage,IDirect3DBaseTexture9* pTexture);

	HRESULT TOGLMETHODCALLTYPE GetTexture(DWORD Stage,IDirect3DBaseTexture9** ppTexture);

	// render targets, color and depthstencil, surfaces, blit
	HRESULT TOGLMETHODCALLTYPE CreateRenderTarget(UINT Width,UINT Height,D3DFORMAT Format,D3DMULTISAMPLE_TYPE MultiSample,DWORD MultisampleQuality,BOOL Lockable,IDirect3DSurface9** ppSurface,VD3DHANDLE* pSharedHandle, char *debugLabel=NULL);
	HRESULT TOGLMETHODCALLTYPE SetRenderTarget(DWORD RenderTargetIndex,IDirect3DSurface9* pRenderTarget);
	HRESULT TOGLMETHODCALLTYPE GetRenderTarget(DWORD RenderTargetIndex,IDirect3DSurface9** ppRenderTarget);

	HRESULT TOGLMETHODCALLTYPE CreateOffscreenPlainSurface(UINT Width,UINT Height,D3DFORMAT Format,D3DPOOL Pool,IDirect3DSurface9** ppSurface,VD3DHANDLE* pSharedHandle);

	HRESULT TOGLMETHODCALLTYPE CreateDepthStencilSurface(UINT Width,UINT Height,D3DFORMAT Format,D3DMULTISAMPLE_TYPE MultiSample,DWORD MultisampleQuality,BOOL Discard,IDirect3DSurface9** ppSurface,VD3DHANDLE* pSharedHandle);
	HRESULT TOGLMETHODCALLTYPE SetDepthStencilSurface(IDirect3DSurface9* pNewZStencil);
	HRESULT TOGLMETHODCALLTYPE GetDepthStencilSurface(IDirect3DSurface9** ppZStencilSurface);

	HRESULT TOGLMETHODCALLTYPE GetRenderTargetData(IDirect3DSurface9* pRenderTarget,IDirect3DSurface9* pDestSurface);
	HRESULT TOGLMETHODCALLTYPE GetFrontBufferData(UINT iSwapChain,IDirect3DSurface9* pDestSurface);
	HRESULT TOGLMETHODCALLTYPE StretchRect(IDirect3DSurface9* pSourceSurface,CONST RECT* pSourceRect,IDirect3DSurface9* pDestSurface,CONST RECT* pDestRect,D3DTEXTUREFILTERTYPE Filter);

	// pixel shaders
	HRESULT TOGLMETHODCALLTYPE CreatePixelShader(CONST DWORD* pFunction,IDirect3DPixelShader9** ppShader, const char *pShaderName, char *debugLabel = NULL, const uint32 *pCentroidMask = NULL );

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetPixelShader(IDirect3DPixelShader9* pShader);
	HRESULT TOGLMETHODCALLTYPE SetPixelShaderNonInline(IDirect3DPixelShader9* pShader);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetPixelShaderConstantF(UINT StartRegister,CONST float* pConstantData,UINT Vector4fCount);
	HRESULT TOGLMETHODCALLTYPE SetPixelShaderConstantFNonInline(UINT StartRegister,CONST float* pConstantData,UINT Vector4fCount);

	HRESULT TOGLMETHODCALLTYPE SetPixelShaderConstantB(UINT StartRegister,CONST BOOL* pConstantData,UINT  BoolCount);
	HRESULT TOGLMETHODCALLTYPE SetPixelShaderConstantI(UINT StartRegister,CONST int* pConstantData,UINT Vector4iCount);

	// vertex shaders
	HRESULT TOGLMETHODCALLTYPE CreateVertexShader(CONST DWORD* pFunction,IDirect3DVertexShader9** ppShader, const char *pShaderName, char *debugLabel = NULL);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetVertexShader(IDirect3DVertexShader9* pShader);
	HRESULT TOGLMETHODCALLTYPE SetVertexShaderNonInline(IDirect3DVertexShader9* pShader);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetVertexShaderConstantF(UINT StartRegister,CONST float* pConstantData,UINT Vector4fCount);
	HRESULT TOGLMETHODCALLTYPE SetVertexShaderConstantFNonInline(UINT StartRegister,CONST float* pConstantData,UINT Vector4fCount);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetVertexShaderConstantB(UINT StartRegister,CONST BOOL* pConstantData,UINT  BoolCount);
	HRESULT TOGLMETHODCALLTYPE SetVertexShaderConstantBNonInline(UINT StartRegister,CONST BOOL* pConstantData,UINT  BoolCount);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetVertexShaderConstantI(UINT StartRegister,CONST int* pConstantData,UINT Vector4iCount);
	HRESULT TOGLMETHODCALLTYPE SetVertexShaderConstantINonInline(UINT StartRegister,CONST int* pConstantData,UINT Vector4iCount);

	// POSIX only - preheating for a specific vertex/pixel shader pair
	HRESULT TOGLMETHODCALLTYPE LinkShaderPair( IDirect3DVertexShader9* vs, IDirect3DPixelShader9* ps );
	HRESULT TOGLMETHODCALLTYPE ValidateShaderPair( IDirect3DVertexShader9* vs, IDirect3DPixelShader9* ps );
	HRESULT TOGLMETHODCALLTYPE QueryShaderPair( int index, GLMShaderPairInfo *infoOut );

	// vertex buffers
	HRESULT TOGLMETHODCALLTYPE CreateVertexDeclaration(CONST D3DVERTEXELEMENT9* pVertexElements,IDirect3DVertexDeclaration9** ppDecl);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetVertexDeclaration(IDirect3DVertexDeclaration9* pDecl);
	HRESULT TOGLMETHODCALLTYPE SetVertexDeclarationNonInline(IDirect3DVertexDeclaration9* pDecl);

	HRESULT TOGLMETHODCALLTYPE SetFVF(DWORD FVF);
	HRESULT TOGLMETHODCALLTYPE GetFVF(DWORD* pFVF);

	HRESULT CreateVertexBuffer(UINT Length,DWORD Usage,DWORD FVF,D3DPOOL Pool,IDirect3DVertexBuffer9** ppVertexBuffer,VD3DHANDLE* pSharedHandle);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetStreamSource(UINT StreamNumber,IDirect3DVertexBuffer9* pStreamData,UINT OffsetInBytes,UINT Stride);
	HRESULT SetStreamSourceNonInline(UINT StreamNumber,IDirect3DVertexBuffer9* pStreamData,UINT OffsetInBytes,UINT Stride);

	// index buffers
	HRESULT TOGLMETHODCALLTYPE CreateIndexBuffer(UINT Length,DWORD Usage,D3DFORMAT Format,D3DPOOL Pool,IDirect3DIndexBuffer9** ppIndexBuffer,VD3DHANDLE* pSharedHandle);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetIndices(IDirect3DIndexBuffer9* pIndexData);
	HRESULT TOGLMETHODCALLTYPE SetIndicesNonInline(IDirect3DIndexBuffer9* pIndexData);

	// State management.
	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetRenderStateInline(D3DRENDERSTATETYPE State,DWORD Value);
	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetRenderStateConstInline(D3DRENDERSTATETYPE State,DWORD Value);
	HRESULT TOGLMETHODCALLTYPE SetRenderState(D3DRENDERSTATETYPE State,DWORD Value);

	FORCEINLINE HRESULT TOGLMETHODCALLTYPE SetSamplerState(DWORD Sampler,D3DSAMPLERSTATETYPE Type,DWORD Value);
	HRESULT TOGLMETHODCALLTYPE SetSamplerStateNonInline(DWORD Sampler,D3DSAMPLERSTATETYPE Type,DWORD Value);

	FORCEINLINE void TOGLMETHODCALLTYPE SetSamplerStates(DWORD Sampler, DWORD AddressU, DWORD AddressV, DWORD AddressW, DWORD MinFilter, DWORD MagFilter, DWORD MipFilter, DWORD MinLod, float LodBias );
	void TOGLMETHODCALLTYPE SetSamplerStatesNonInline(DWORD Sampler, DWORD AddressU, DWORD AddressV, DWORD AddressW, DWORD MinFilter, DWORD MagFilter, DWORD MipFilter, DWORD MinLod, float LodBias );

#ifdef OSX
	// required for 10.6 support
	HRESULT TOGLMETHODCALLTYPE FlushIndexBindings(void);
	HRESULT	TOGLMETHODCALLTYPE FlushVertexBindings(uint baseVertexIndex);
#endif

	// Draw.
	HRESULT TOGLMETHODCALLTYPE DrawPrimitive(D3DPRIMITIVETYPE PrimitiveType,UINT StartVertex,UINT PrimitiveCount);
	HRESULT TOGLMETHODCALLTYPE DrawIndexedPrimitive(D3DPRIMITIVETYPE PrimitiveType,INT BaseVertexIndex,UINT MinVertexIndex,UINT NumVertices,UINT startIndex,UINT primCount);
	HRESULT TOGLMETHODCALLTYPE DrawIndexedPrimitiveUP(D3DPRIMITIVETYPE PrimitiveType,UINT MinVertexIndex,UINT NumVertices,UINT PrimitiveCount,CONST void* pIndexData,D3DFORMAT IndexDataFormat,CONST void* pVertexStreamZeroData,UINT VertexStreamZeroStride);

	// misc
	BOOL TOGLMETHODCALLTYPE ShowCursor(BOOL bShow);
	HRESULT TOGLMETHODCALLTYPE ValidateDevice(DWORD* pNumPasses);
	HRESULT TOGLMETHODCALLTYPE SetMaterial(CONST D3DMATERIAL9* pMaterial);
	HRESULT TOGLMETHODCALLTYPE LightEnable(DWORD Index,BOOL Enable);
	HRESULT TOGLMETHODCALLTYPE SetScissorRect(CONST RECT* pRect);
	HRESULT TOGLMETHODCALLTYPE CreateQuery(D3DQUERYTYPE Type,IDirect3DQuery9** ppQuery);
	HRESULT TOGLMETHODCALLTYPE GetDeviceCaps(D3DCAPS9* pCaps);
	HRESULT TOGLMETHODCALLTYPE TestCooperativeLevel();
	HRESULT TOGLMETHODCALLTYPE EvictManagedResources();
	HRESULT TOGLMETHODCALLTYPE SetLight(DWORD Index,CONST D3DLIGHT9*);
	void TOGLMETHODCALLTYPE SetGammaRamp(UINT iSwapChain,DWORD Flags,CONST D3DGAMMARAMP* pRamp);

	void TOGLMETHODCALLTYPE SaveGLState();
	void TOGLMETHODCALLTYPE RestoreGLState();

	HRESULT TOGLMETHODCALLTYPE SetClipPlane(DWORD Index,CONST float* pPlane);

	//
	// **** FIXED FUNCTION STUFF - none of it is used with shaders.
	//
	HRESULT TOGLMETHODCALLTYPE SetTransform(D3DTRANSFORMSTATETYPE State,CONST D3DMATRIX* pMatrix);
	HRESULT TOGLMETHODCALLTYPE SetTextureStageState(DWORD Stage,D3DTEXTURESTAGESTATETYPE Type,DWORD Value);

	void TOGLMETHODCALLTYPE AcquireThreadOwnership( );
	void TOGLMETHODCALLTYPE ReleaseThreadOwnership( );
	inline uintp TOGLMETHODCALLTYPE GetCurrentOwnerThreadId() const { return m_nCurOwnerThreadId; }

	FORCEINLINE void TOGLMETHODCALLTYPE SetMaxUsedVertexShaderConstantsHint( uint nMaxReg );
	void TOGLMETHODCALLTYPE SetMaxUsedVertexShaderConstantsHintNonInline( uint nMaxReg );

private:
	IDirect3DDevice9( const IDirect3DDevice9& );
	IDirect3DDevice9& operator= ( const IDirect3DDevice9& );

	void InitStates();
	HRESULT CreateDefaultSurfaces();
	void ReleaseDefaultSurfaces();

	// response to retired objects (when refcount goes to zero and they self-delete..)
	void ReleasedVertexDeclaration( IDirect3DVertexDeclaration9 *pDecl );
	void ReleasedTexture( IDirect3DBaseTexture9 *baseTex );
	void ReleasedSurface( IDirect3DSurface9 *surface );
	void ReleasedPixelShader( IDirect3DPixelShader9 *pixelShader );
	void ReleasedVertexShader( IDirect3DVertexShader9 *vertexShader );
	void ReleasedVertexBuffer( IDirect3DVertexBuffer9 *vertexBuffer );
	void ReleasedIndexBuffer( IDirect3DIndexBuffer9 *indexBuffer );
	void ReleasedQuery( IDirect3DQuery9 *query );

public:
	IDirect3DDevice9Params	m_params;						// mirror of the creation inputs
	SourceD3D9Handle		m_dev;							// the Metal device

private:
	IDirect3DSurface9			*m_pRenderTargets[SOURCE_D3D9_RENDER_TARGETS];
	IDirect3DSurface9			*m_pDepthStencil;

	IDirect3DSurface9			*m_pDefaultColorSurface;
	IDirect3DSurface9			*m_pDefaultDepthStencilSurface;

	IDirect3DVertexDeclaration9	*m_pVertDecl;
	IDirect3DVertexBuffer9		*m_streamBuffers[SOURCE_D3D9_STREAMS];
	IDirect3DIndexBuffer9		*m_pIndices;

	IDirect3DVertexShader9		*m_vertexShader;
	IDirect3DPixelShader9		*m_pixelShader;

	IDirect3DBaseTexture9		*m_textures[SOURCE_D3D9_SAMPLERS];

	uintp						m_nCurOwnerThreadId;

	// Everything a draw depends on. Handed to the Metal device by pointer.
	SourceD3D9State				m_state;
};

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetSamplerState( DWORD Sampler, D3DSAMPLERSTATETYPE Type, DWORD Value )
{
	// Vertex texture and displacement samplers are numbered from 256 and
	// nothing in the shipped shaders reads one.
	if ( Sampler < SOURCE_D3D9_SAMPLERS && (uint)Type < SOURCE_D3D9_SAMPLER_STATES )
		m_state.sampler_states[Sampler][Type] = Value;
	return S_OK;
}

FORCEINLINE void TOGLMETHODCALLTYPE IDirect3DDevice9::SetSamplerStates(
	DWORD Sampler, DWORD AddressU, DWORD AddressV, DWORD AddressW,
	DWORD MinFilter, DWORD MagFilter, DWORD MipFilter, DWORD MinLod, float LodBias )
{
	if ( Sampler >= SOURCE_D3D9_SAMPLERS )
		return;
	uint32_t *pStates = m_state.sampler_states[Sampler];
	pStates[D3DSAMP_ADDRESSU] = AddressU;
	pStates[D3DSAMP_ADDRESSV] = AddressV;
	pStates[D3DSAMP_ADDRESSW] = AddressW;
	pStates[D3DSAMP_MINFILTER] = MinFilter;
	pStates[D3DSAMP_MAGFILTER] = MagFilter;
	pStates[D3DSAMP_MIPFILTER] = MipFilter;
	pStates[D3DSAMP_MAXMIPLEVEL] = MinLod;
	memcpy( &pStates[D3DSAMP_MIPMAPLODBIAS], &LodBias, sizeof( LodBias ) );
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetTexture(DWORD Stage,IDirect3DBaseTexture9* pTexture)
{
	if ( Stage < SOURCE_D3D9_SAMPLERS )
	{
		m_textures[Stage] = pTexture;
		m_state.textures[Stage] = pTexture ? pTexture->m_tex : 0;
	}
	return S_OK;
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetRenderStateInline( D3DRENDERSTATETYPE State, DWORD Value )
{
	if ( (uint)State < SOURCE_D3D9_RENDER_STATES )
		m_state.render_states[State] = Value;
	return S_OK;
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetRenderStateConstInline( D3DRENDERSTATETYPE State, DWORD Value )
{
	return SetRenderStateInline( State, Value );
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetIndices(IDirect3DIndexBuffer9* pIndexData)
{
	m_pIndices = pIndexData;
	m_state.index_buffer = pIndexData ? pIndexData->m_idxBuffer : 0;
	return S_OK;
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetStreamSource(UINT StreamNumber,IDirect3DVertexBuffer9* pStreamData,UINT OffsetInBytes,UINT Stride)
{
	if ( StreamNumber < SOURCE_D3D9_STREAMS )
	{
		m_streamBuffers[StreamNumber] = pStreamData;
		SourceD3D9Stream &stream = m_state.streams[StreamNumber];
		stream.buffer = pStreamData ? pStreamData->m_vtxBuffer : 0;
		stream.offset = OffsetInBytes;
		stream.stride = Stride;
	}
	return S_OK;
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetVertexShaderConstantF(UINT StartRegister,CONST float* pConstantData,UINT Vector4fCount)
{
	if ( StartRegister < SOURCE_D3D9_VS_FLOAT_CONSTANTS )
	{
		UINT nCount = MIN( Vector4fCount, SOURCE_D3D9_VS_FLOAT_CONSTANTS - StartRegister );
		memcpy( m_state.vs_floats[StartRegister], pConstantData, nCount * 4 * sizeof( float ) );
	}
	return S_OK;
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetVertexShaderConstantB(UINT StartRegister,CONST BOOL* pConstantData,UINT BoolCount)
{
	for ( UINT i = 0; i < BoolCount && ( StartRegister + i ) < 32; ++i )
	{
		const uint32_t nBit = 1u << ( StartRegister + i );
		if ( pConstantData[i] )
			m_state.vs_bools |= nBit;
		else
			m_state.vs_bools &= ~nBit;
	}
	return S_OK;
}

FORCEINLINE HRESULT IDirect3DDevice9::SetVertexShaderConstantI(UINT StartRegister,CONST int* pConstantData,UINT Vector4iCount)
{
	if ( StartRegister < SOURCE_D3D9_INT_CONSTANTS )
	{
		UINT nCount = MIN( Vector4iCount, SOURCE_D3D9_INT_CONSTANTS - StartRegister );
		memcpy( m_state.vs_ints[StartRegister], pConstantData, nCount * 4 * sizeof( int ) );
	}
	return S_OK;
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetPixelShaderConstantF(UINT StartRegister,CONST float* pConstantData,UINT Vector4fCount)
{
	if ( StartRegister < SOURCE_D3D9_PS_FLOAT_CONSTANTS )
	{
		UINT nCount = MIN( Vector4fCount, SOURCE_D3D9_PS_FLOAT_CONSTANTS - StartRegister );
		memcpy( m_state.ps_floats[StartRegister], pConstantData, nCount * 4 * sizeof( float ) );
	}
	return S_OK;
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetVertexShader(IDirect3DVertexShader9* pShader)
{
	m_vertexShader = pShader;
	m_state.vertex_shader = pShader ? pShader->m_vtxProgram : 0;
	return S_OK;
}

FORCEINLINE HRESULT TOGLMETHODCALLTYPE IDirect3DDevice9::SetPixelShader(IDirect3DPixelShader9* pShader)
{
	m_pixelShader = pShader;
	m_state.pixel_shader = pShader ? pShader->m_pixProgram : 0;
	return S_OK;
}

FORCEINLINE HRESULT IDirect3DDevice9::SetVertexDeclaration(IDirect3DVertexDeclaration9* pDecl)
{
	m_pVertDecl = pDecl;
	m_state.vertex_declaration = pDecl ? pDecl->m_decl : 0;
	return S_OK;
}

FORCEINLINE void IDirect3DDevice9::SetMaxUsedVertexShaderConstantsHint( uint nMaxReg )
{
	// The translated shader already says how many constants it reads.
}

// ------------------------------------------------------------------------------------------------------------------------------ //
// D3DX
// ------------------------------------------------------------------------------------------------------------------------------ //
struct ID3DXInclude
{
	virtual HRESULT Open(D3DXINCLUDE_TYPE IncludeType, LPCSTR pFileName, LPCVOID pParentData, LPCVOID *ppData, UINT *pBytes);
	virtual HRESULT Close(LPCVOID pData);
};
typedef ID3DXInclude* LPD3DXINCLUDE;

struct TOGL_CLASS ID3DXBuffer : public IUnknown
{
	void* GetBufferPointer();
	DWORD GetBufferSize();
};

typedef ID3DXBuffer* LPD3DXBUFFER;

class ID3DXConstantTable : public IUnknown
{
};
typedef ID3DXConstantTable* LPD3DXCONSTANTTABLE;

TOGL_INTERFACE const char* D3DXGetPixelShaderProfile( IDirect3DDevice9 *pDevice );

TOGL_INTERFACE D3DXMATRIX* D3DXMatrixMultiply( D3DXMATRIX *pOut, CONST D3DXMATRIX *pM1, CONST D3DXMATRIX *pM2 );
TOGL_INTERFACE D3DXVECTOR3* D3DXVec3TransformCoord( D3DXVECTOR3 *pOut, CONST D3DXVECTOR3 *pV, CONST D3DXMATRIX *pM );

TOGL_INTERFACE HRESULT D3DXCreateMatrixStack( DWORD Flags, LPD3DXMATRIXSTACK* ppStack);
TOGL_INTERFACE void D3DXMatrixIdentity( D3DXMATRIX * );

TOGL_INTERFACE D3DXINLINE D3DXVECTOR3* D3DXVec3Subtract( D3DXVECTOR3 *pOut, CONST D3DXVECTOR3 *pV1, CONST D3DXVECTOR3 *pV2 )
{
	pOut->x = pV1->x - pV2->x;
	pOut->y = pV1->y - pV2->y;
	pOut->z = pV1->z - pV2->z;
	return pOut;
}

TOGL_INTERFACE D3DXINLINE D3DXVECTOR3* D3DXVec3Cross( D3DXVECTOR3 *pOut, CONST D3DXVECTOR3 *pV1, CONST D3DXVECTOR3 *pV2 )
{
	D3DXVECTOR3 v;

	v.x = pV1->y * pV2->z - pV1->z * pV2->y;
	v.y = pV1->z * pV2->x - pV1->x * pV2->z;
	v.z = pV1->x * pV2->y - pV1->y * pV2->x;

	*pOut = v;
	return pOut;
}

TOGL_INTERFACE D3DXINLINE FLOAT D3DXVec3Dot( CONST D3DXVECTOR3 *pV1, CONST D3DXVECTOR3 *pV2 )
{
	return pV1->x * pV2->x + pV1->y * pV2->y + pV1->z * pV2->z;
}

TOGL_INTERFACE D3DXMATRIX* D3DXMatrixInverse( D3DXMATRIX *pOut, FLOAT *pDeterminant, CONST D3DXMATRIX *pM );
TOGL_INTERFACE D3DXMATRIX* D3DXMatrixTranspose( D3DXMATRIX *pOut, CONST D3DXMATRIX *pM );
TOGL_INTERFACE D3DXPLANE* D3DXPlaneNormalize( D3DXPLANE *pOut, CONST D3DXPLANE *pP);
TOGL_INTERFACE D3DXVECTOR4* D3DXVec4Transform( D3DXVECTOR4 *pOut, CONST D3DXVECTOR4 *pV, CONST D3DXMATRIX *pM );
TOGL_INTERFACE D3DXVECTOR4* D3DXVec4Normalize( D3DXVECTOR4 *pOut, CONST D3DXVECTOR4 *pV );
TOGL_INTERFACE D3DXMATRIX* D3DXMatrixTranslation( D3DXMATRIX *pOut, FLOAT x, FLOAT y, FLOAT z );
TOGL_INTERFACE D3DXMATRIX* D3DXMatrixOrthoOffCenterRH( D3DXMATRIX *pOut, FLOAT l, FLOAT r, FLOAT b, FLOAT t, FLOAT zn,FLOAT zf );
TOGL_INTERFACE D3DXMATRIX* D3DXMatrixPerspectiveRH( D3DXMATRIX *pOut, FLOAT w, FLOAT h, FLOAT zn, FLOAT zf );
TOGL_INTERFACE D3DXMATRIX* D3DXMatrixPerspectiveOffCenterRH( D3DXMATRIX *pOut, FLOAT l, FLOAT r, FLOAT b, FLOAT t, FLOAT zn, FLOAT zf );

// Transform a plane by a matrix.  The vector (a,b,c) must be normal.
// M should be the inverse transpose of the transformation desired.
TOGL_INTERFACE D3DXPLANE* D3DXPlaneTransform( D3DXPLANE *pOut, CONST D3DXPLANE *pP, CONST D3DXMATRIX *pM );

TOGL_INTERFACE IDirect3D9 *Direct3DCreate9(UINT SDKVersion);

TOGL_INTERFACE void D3DPERF_SetOptions( DWORD dwOptions );

TOGL_INTERFACE HRESULT D3DXCompileShader(
	LPCSTR                          pSrcData,
	UINT                            SrcDataLen,
	CONST D3DXMACRO*                pDefines,
	LPD3DXINCLUDE                   pInclude,
	LPCSTR                          pFunctionName,
	LPCSTR                          pProfile,
	DWORD                           Flags,
	LPD3DXBUFFER*                   ppShader,
	LPD3DXBUFFER*                   ppErrorMsgs,
	LPD3DXCONSTANTTABLE*            ppConstantTable);

// fake D3D usage constant for SRGB tex creation
#define D3DUSAGE_TEXTURE_SRGB (0x80000000L)

#endif // TOMETAL_DXABSTRACT_H
