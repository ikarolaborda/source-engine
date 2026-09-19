//========= Copyright Valve Corporation, All rights reserved. ============//
//
// Purpose: master for refresh, status bar, console, chat, notify, etc
//
//=====================================================================================//

#include "render_pch.h"
#include "client.h"
#include "console.h"
#include "screen.h"
#include "sound.h"
#include "sbar.h"
#include "debugoverlay.h"
#include "ivguicenterprint.h"
#include "cdll_int.h"
#include "gl_matsysiface.h"
#include "cdll_engine_int.h"
#include "demo.h"
#include "cl_main.h"
#include "vgui_baseui_interface.h"
#include "tier0/vcrmode.h"
#include "con_nprint.h"
#include "sys_mainwind.h"
#include "ivideomode.h"
#include "lightcache.h"
#include "toolframework/itoolframework.h"
#include "matchmaking.h"
#include "datacache/idatacache.h"
#include "sys_dll.h"
#if defined( REPLAY_ENABLED )
#include "replay_internal.h"
#endif
#include "tier0/vprof.h"
#if defined( SOURCE_RUST_ENGINE ) && defined( OSX )
#include "../appframework/rust_engine_bridge.h"
#endif

// memdbgon must be the last include file in a .cpp file!!!
#include "tier0/memdbgon.h"

// In other C files.
extern bool V_CheckGamma( void );
extern void	V_RenderView( void );
extern void V_RenderVGuiOnly( void );

bool		scr_initialized;		// ready to draw
bool		scr_disabled_for_loading;
bool		scr_drawloading;
int			scr_nextdrawtick;		// A hack to let things settle on reload/reconnect

//-----------------------------------------------------------------------------
// Purpose: 
//-----------------------------------------------------------------------------
void SCR_Init (void)
{
	scr_initialized = true;
}

//-----------------------------------------------------------------------------
// Purpose: 
//-----------------------------------------------------------------------------
void SCR_Shutdown( void )
{
	scr_initialized = false;
}

//-----------------------------------------------------------------------------
// Purpose: starts loading
//-----------------------------------------------------------------------------
void SCR_BeginLoadingPlaque( void )
{
	if ( !scr_drawloading )
	{
		// make sure game UI is allowed to show (gets disabled if chat window is up)
		EngineVGui()->SetNotAllowedToShowGameUI( false );

		// force QMS to serialize during loading
		Host_AllowQueuedMaterialSystem( false );

		scr_drawloading = true;

		S_StopAllSounds( true );
		S_OnLoadScreen( true );

		g_pFileSystem->AsyncFinishAll();
		g_pMDLCache->FinishPendingLoads();

		// redraw with no console and the loading plaque
		Con_ClearNotify();
		SCR_CenterStringOff();

		// NULL HudText clears HudMessage system
		if ( g_ClientDLL )
		{
			g_ClientDLL->HudText( NULL );
		}

		// let the UI know we're starting loading
		EngineVGui()->OnLevelLoadingStarted();

		// Don't run any more simulation on the client!!!
		g_ClientGlobalVariables.frametime = 0.0f;

		host_framecount++;
		g_ClientGlobalVariables.framecount = host_framecount;
		// Ensure the screen is painted to reflect the loading state
		SCR_UpdateScreen();
		host_framecount++;
		g_ClientGlobalVariables.framecount = host_framecount;
		SCR_UpdateScreen();

		g_ClientGlobalVariables.frametime = cl.GetFrameTime();

		scr_disabled_for_loading = true;
	}
}

//-----------------------------------------------------------------------------
// Purpose: finished loading
//-----------------------------------------------------------------------------
void SCR_EndLoadingPlaque( void )
{
	if ( scr_drawloading )
	{
		// let the UI know we're finished
		EngineVGui()->OnLevelLoadingFinished();
		S_OnLoadScreen( false );
	}
	else if ( gfExtendedError )
	{
		if ( IsPC() )
		{
			EngineVGui()->ShowErrorMessage();
		}
	}

	g_pMatchmaking->OnLevelLoadingFinished();

	scr_disabled_for_loading = false;
	scr_drawloading = false;
}

//-----------------------------------------------------------------------------
// Places TCR required defective media message and halts 
//-----------------------------------------------------------------------------
#ifdef _XBOX
void SCR_FatalDiskError()
{
	EngineVGui()->OnDiskError();
	while ( 1 )
	{
		// run the minimal frame to update and paint
		EngineVGui()->Simulate();
		V_RenderVGuiOnly();
	}
}
#endif

//-----------------------------------------------------------------------------
// Purpose: 
//-----------------------------------------------------------------------------
void SCR_CenterPrint (char *str)
{
	if ( !centerprint )
		return;

	centerprint->ColorPrint( 255, 255, 255, 0, str );
}

//-----------------------------------------------------------------------------
// Purpose: 
//-----------------------------------------------------------------------------
void SCR_CenterStringOff( void )
{
	if ( !centerprint )
		return;

	centerprint->Clear();
}

//-----------------------------------------------------------------------------
// Purpose: 
//-----------------------------------------------------------------------------
inline void SCR_ShowVCRPlaybackAmount()
{
	if ( VCRGetMode() != VCR_Playback || !g_bShowVCRPlaybackDisplay )
		return;

	con_nprint_t info;
	info.index = 20;
	info.time_to_live = 0.01;
	info.color[0] = info.color[1] = info.color[2] = 1;
	info.fixed_width_font = false;

	double flCurPercent = VCRGetPercentCompleted();
	Con_NXPrintf( &info, "VCR Playback: %.2f percent, frame %d", flCurPercent * 100.0, host_framecount );
	info.index++;

	Con_NXPrintf( &info, "'+' to speed up, '-' to slow down [current sleep: %d]", g_iVCRPlaybackSleepInterval );
	info.index++;
	
	Con_NXPrintf( &info, "'p' to pause, 's' to single step, 'r' to resume" );
	info.index++;
	
	Con_NXPrintf( &info, "'d' to toggle this display" );
	info.index++;
	
	Con_NXPrintf( &info, "'q' to quit" );
}

#if defined( SOURCE_RUST_ENGINE ) && defined( OSX )
//-----------------------------------------------------------------------------
// Purpose: Draws the map through the Rust Metal renderer.
//
//  This runs where the old renderer ran, once per frame with the engine's
//  view already set up. The renderer loaded the map itself, so all it is
//  given is where the player is looking from; what of the map that reaches
//  is the renderer's own decision.
//
//  The map is loaded on the first frame of a level rather than at load
//  time, because that is the earliest point at which both the level file
//  name and the presenter are certain to exist.
//-----------------------------------------------------------------------------
static void SCR_PresentRustWorld( void )
{
	if ( source_rust_bridge_presenter() == 0 )
		return;

	// The server's own world model, rather than the client's level file
	// name: the name is what the client displays on its scoreboard and is
	// empty until it has parsed the server info, while the world model is
	// set as the level loads, which is when there is something to draw.
	if ( host_state.worldmodel == NULL )
		return;
	const char *level = modelloader->GetName( host_state.worldmodel );
	if ( !level || !level[ 0 ] )
		return;

	static char s_LoadedMap[ 128 ] = { 0 };
	static bool s_SceneReady = false;
	SourceAbiWorldDraw drawn = {};
	if ( Q_stricmp( s_LoadedMap, level ) != 0 )
	{
		// Recorded before the attempt so that a map which cannot be loaded
		// is reported once rather than on every frame of the level.
		Q_strncpy( s_LoadedMap, level, sizeof( s_LoadedMap ) );
		const SourceAbiStatus status = source_rust_bridge_scene_load( level,
			Q_strlen( level ), &drawn );
		s_SceneReady = ( status == SOURCE_ABI_OK );
		if ( !s_SceneReady )
		{
			Warning( "Rust Metal scene load failed for %s (status %d)\n", level, status );
			return;
		}
		Msg( "Rust Metal scene: %llu triangles, %llu materials, %llu props (%s)\n",
			(unsigned long long)drawn.map_triangles,
			(unsigned long long)drawn.materials,
			(unsigned long long)drawn.props, level );
	}
	if ( !s_SceneReady )
		return;

	// The last view the engine set up, which is the one just rendered. The
	// accessors in render.h assert on being inside a view push, and this
	// runs just after one has been popped.
	extern Vector g_CurrentViewOrigin;
	extern Vector g_CurrentViewForward;
	QAngle viewAngles;
	VectorAngles( g_CurrentViewForward, viewAngles );
	const float position[ 3 ] = { g_CurrentViewOrigin.x, g_CurrentViewOrigin.y,
		g_CurrentViewOrigin.z };
	const float angles[ 3 ] = { viewAngles.x, viewAngles.y, viewAngles.z };
	source_rust_bridge_scene_present( position, angles, &drawn );

	// One line a second rather than one a frame, which is enough to show
	// the view moving and what it costs without filling the log.
	static double s_NextReport = 0.0;
	const double now = Plat_FloatTime();
	if ( now >= s_NextReport )
	{
		s_NextReport = now + 1.0;
		Msg( "RUST_METAL_FRAME batches=%llu triangles=%llu props=%llu of=%llu "
			"eye=%.0f %.0f %.0f angles=%.0f %.0f\n",
			(unsigned long long)drawn.batches, (unsigned long long)drawn.triangles,
			(unsigned long long)drawn.prop_triangles,
			(unsigned long long)drawn.map_triangles, position[ 0 ], position[ 1 ],
			position[ 2 ], angles[ 0 ], angles[ 1 ] );
	}
}
#endif

//-----------------------------------------------------------------------------
// Purpose: This is called every frame, and can also be called explicitly to flush
//  text to the screen.
//-----------------------------------------------------------------------------
void SCR_UpdateScreen( void )
{
	tmZone( TELEMETRY_LEVEL0, TMZF_NONE, "%s", __FUNCTION__ );

	R_StudioCheckReinitLightingCache();
	
	// Always force the Gamma Table to be rebuilt. Otherwise,
	// we'll load textures with an all white gamma lookup table.
	V_CheckGamma();

	// This is a HACK to let things settle for a bit on level start
	// NOTE: If you remove scr_nextdrawtick, remove it from enginetool.cpp too
	if ( scr_nextdrawtick != 0 )
	{
		if ( host_tickcount < scr_nextdrawtick )
			return;

		scr_nextdrawtick = 0;
	}

	if ( scr_disabled_for_loading )
	{
		if ( !Host_IsSinglePlayerGame() )
		{
			V_RenderVGuiOnly();
		}
		return;
	}

	if ( !scr_initialized || !con_initialized )
	{
		// not initialized yet
		return;				
	}

	SCR_ShowVCRPlaybackAmount();

	// Let demo system overwrite view origin/angles during playback
	if ( demoplayer->IsPlayingBack() )
	{
		demoplayer->InterpolateViewpoint();
	}

	materials->BeginFrame( host_frametime );
	{
		tmZone( TELEMETRY_LEVEL0, TMZF_NONE, "EngineVGui_Simulate" );
		EngineVGui()->Simulate();
	}

	ClientDLL_FrameStageNotify( FRAME_RENDER_START );

	// Simulation meant to occur before any views are rendered
	// This needs to happen before the client DLL is called because the client DLL depends on 
	// some of the setup in FRAME_RENDER_START.
	{
		tmZone( TELEMETRY_LEVEL0, TMZF_NONE, "FrameBegin" );
	
		g_EngineRenderer->FrameBegin();
		toolframework->RenderFrameBegin();
	}

	cl.UpdateAreaBits_BackwardsCompatible();
	
	Shader_BeginRendering();
				
	// Draw world, etc.
	V_RenderView();

#if defined( SOURCE_RUST_ENGINE ) && defined( OSX )
	SCR_PresentRustWorld();
#endif

	CL_TakeSnapshotAndSwap();	   
	
#if defined( REPLAY_ENABLED )
	if ( g_pReplay )
	{
		g_pReplay->CL_Render();
	}
#endif

	ClientDLL_FrameStageNotify( FRAME_RENDER_END );

	{
		tmZone( TELEMETRY_LEVEL0, TMZF_NONE, "FrameEnd" );

		toolframework->RenderFrameEnd();

		g_EngineRenderer->FrameEnd();
	}

	// moved dynamic model update here because this takes the materials lock
	// and materials->EndFrame() is where we will synchronize anyway.
	// Moved here to leave as much of the frame as possible to overlap threads in the case
	// where we actually have models to load here
	{
		tmZone( TELEMETRY_LEVEL0, TMZF_NONE, "modelloader->UpdateDynamicModels" );
		VPROF( "UpdateDynamicModels" );
		CMDLCacheCriticalSection critsec( g_pMDLCache );
		modelloader->UpdateDynamicModels();
	}

	{
		tmZone( TELEMETRY_LEVEL0, TMZF_NONE, "materials_EndFrame" );

		materials->EndFrame();
	}
}

