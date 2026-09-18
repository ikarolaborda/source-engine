//========= Copyright Valve Corporation, All rights reserved. ============//
//
// Purpose: 
//
// $NoKeywords: $
//=============================================================================//


#include "server_pch.h"
#include <eiface.h>
#include <dt_send.h>
#include <utllinkedlist.h>
#include "tier0/etwprof.h"
#include "dt_send_eng.h"
#include "dt.h"
#include "net_synctags.h"
#include "dt_instrumentation_server.h"
#include "LocalNetworkBackdoor.h"
#include "ents_shared.h"
#include "hltvserver.h"
#include "replayserver.h"
#include "tier0/vcrmode.h"
#include "framesnapshot.h"
#if defined( SOURCE_RUST_ENGINE )
#include "../appframework/rust_engine_bridge.h"
#endif


// memdbgon must be the last include file in a .cpp file!!!
#include "tier0/memdbgon.h"

extern ConVar g_CV_DTWatchEnt;

//-----------------------------------------------------------------------------
// Delta timing stuff.
//-----------------------------------------------------------------------------

static ConVar		sv_deltatime( "sv_deltatime", "0", 0, "Enable profiling of CalcDelta calls" );
static ConVar		sv_deltaprint( "sv_deltaprint", "0", 0, "Print accumulated CalcDelta profiling data (only if sv_deltatime is on)" );

#if defined( DEBUG_NETWORKING )
ConVar  sv_packettrace( "sv_packettrace", "1", 0, "For debugging, print entity creation/deletion info to console." );
#endif

class CChangeTrack
{
public:
	char		*m_pName;
	int			m_nChanged;
	int			m_nUnchanged;
	
	CCycleCount	m_Count;
	CCycleCount	m_EncodeCount;
};


static CUtlLinkedList<CChangeTrack*, int> g_Tracks;


// These are the main variables used by the SV_CreatePacketEntities function.
// The function is split up into multiple smaller ones and they pass this structure around.
class CEntityWriteInfo : public CEntityInfo
{
public:
	bf_write		*m_pBuf;
	int				m_nClientEntity;

	PackedEntity	*m_pOldPack;
	PackedEntity	*m_pNewPack;

	// For each entity handled in the to packet, mark that's it has already been deleted if that's the case
	CBitVec<MAX_EDICTS>	m_DeletionFlags;
	
	CFrameSnapshot	*m_pFromSnapshot; // = m_pFrom->GetSnapshot();
	CFrameSnapshot	*m_pToSnapshot; // = m_pTo->GetSnapshot();

	CFrameSnapshot	*m_pBaseline; // the clients baseline

	CBaseServer		*m_pServer;	// the server who writes this entity

	int				m_nFullProps;	// number of properties send as full update (Enter PVS)
	bool			m_bCullProps;	// filter props by clients in recipient lists
	
	/* Some profiling data
	int				m_nTotalGap;
	int				m_nTotalGapCount; */
};

#if defined( SOURCE_RUST_ENGINE )
//-----------------------------------------------------------------------------
// Rust classification of one receiver's entity delta.
//
// Rust decides, per visible entity index, whether the receiver has to create
// the entity, drop it, or may delta against what it already has. Choosing
// between resending and preserving a delta candidate stays here, because that
// comparison needs the packed payloads Rust does not hold.
//-----------------------------------------------------------------------------
static bool s_bRustEntityDeltaAvailable = true;
static bool s_bRustEntityDeltaMarkerPrinted = false;

// Rust also encodes the per-entity header bits that precede each update.
static bool s_bRustDeltaHeaderAvailable = true;
static bool s_bRustDeltaHeaderMarkerPrinted = false;

class CRustEntityDelta
{
public:
	// Returns false when this receiver's delta must fall back to the native
	// classification, either because Rust does not own both snapshots or
	// because Rust rejected the visibility sets.
	bool Build( const CEntityWriteInfo &u );

	bool IsActive() const { return m_bActive; }

	// Returns the classification of one entity index, or NULL when the index
	// takes no part in this delta.
	const SourceAbiSnapshotDelta *Find( int nEntityIndex ) const;

	void Disable() { m_bActive = false; }

private:
	static void CollectVisible( const CFrameSnapshot *pSnapshot,
		const CClientFrame *pFrame, CUtlVector<uint32> &visible );

	bool m_bActive = false;
	CUtlVector<SourceAbiSnapshotDelta> m_Deltas;
};

void CRustEntityDelta::CollectVisible( const CFrameSnapshot *pSnapshot,
	const CClientFrame *pFrame, CUtlVector<uint32> &visible )
{
	visible.RemoveAll();
	visible.EnsureCapacity( pSnapshot->m_nValidEntities );

	// Walk the receiver's transmit bits in ascending order, which is the order
	// the delta contract requires.
	for ( int index = pFrame->transmit_entity.FindNextSetBit( 0 ); index >= 0;
		index = pFrame->transmit_entity.FindNextSetBit( index + 1 ) )
	{
		visible.AddToTail( static_cast<uint32>( index ) );
	}
}

bool CRustEntityDelta::Build( const CEntityWriteInfo &u )
{
	m_bActive = false;
	m_Deltas.RemoveAll();

	if ( !s_bRustEntityDeltaAvailable || u.m_pToSnapshot == NULL ||
		u.m_pToSnapshot->m_nRustSnapshotId == 0 )
		return false;
	if ( u.m_bAsDelta && ( u.m_pFromSnapshot == NULL ||
		u.m_pFromSnapshot->m_nRustSnapshotId == 0 ) )
		return false;

	CUtlVector<uint32> toVisible;
	CollectVisible( u.m_pToSnapshot, u.m_pTo, toVisible );

	CUtlVector<uint32> fromVisible;
	if ( u.m_bAsDelta )
	{
		CollectVisible( u.m_pFromSnapshot, u.m_pFrom, fromVisible );
	}

	// One index can appear on both sides but yields a single entry, so the
	// combined visible count is a safe upper bound.
	m_Deltas.SetCount( toVisible.Count() + fromVisible.Count() );
	uint64_t count = 0;
	const SourceAbiStatus status = source_rust_bridge_snapshot_delta(
		u.m_bAsDelta ? u.m_pFromSnapshot->m_nRustSnapshotId : 0,
		u.m_bAsDelta ? fromVisible.Base() : NULL,
		u.m_bAsDelta ? static_cast<uint64_t>( fromVisible.Count() ) : 0,
		u.m_pToSnapshot->m_nRustSnapshotId, toVisible.Base(),
		static_cast<uint64_t>( toVisible.Count() ), m_Deltas.Base(),
		static_cast<uint64_t>( m_Deltas.Count() ), &count );
	if ( status != SOURCE_ABI_OK || count > static_cast<uint64_t>( m_Deltas.Count() ) )
	{
		m_Deltas.RemoveAll();
		// A rejected visibility set means the Rust and native views disagree
		// about this frame, which the native path has to resolve.
		s_bRustEntityDeltaAvailable = false;
		ConMsg( "Rust entity delta classification disabled: status %d\n", status );
		return false;
	}

	m_Deltas.SetCountNonDestructively( static_cast<int>( count ) );
	m_bActive = true;

	if ( !s_bRustEntityDeltaMarkerPrinted )
	{
		ConMsg( "Rust entity delta classification active: tick %d, %d visible, %d classified\n",
			u.m_pToSnapshot->m_nTickCount, toVisible.Count(), m_Deltas.Count() );
		s_bRustEntityDeltaMarkerPrinted = true;
	}

	return true;
}

const SourceAbiSnapshotDelta *CRustEntityDelta::Find( int nEntityIndex ) const
{
	if ( !m_bActive || nEntityIndex < 0 )
		return NULL;

	// Entries are ascending by entity index.
	int low = 0;
	int high = m_Deltas.Count() - 1;
	while ( low <= high )
	{
		const int middle = low + ( high - low ) / 2;
		const uint32 index = m_Deltas[middle].entity_index;
		if ( index == static_cast<uint32>( nEntityIndex ) )
			return &m_Deltas[middle];
		if ( index < static_cast<uint32>( nEntityIndex ) )
			low = middle + 1;
		else
			high = middle - 1;
	}
	return NULL;
}
#endif // SOURCE_RUST_ENGINE



//-----------------------------------------------------------------------------
// Delta timing helpers.
//-----------------------------------------------------------------------------

CChangeTrack* GetChangeTrack( const char *pName )
{
	FOR_EACH_LL( g_Tracks, i )
	{
		CChangeTrack *pCur = g_Tracks[i];

		if ( stricmp( pCur->m_pName, pName ) == 0 )
			return pCur;
	}

	CChangeTrack *pCur = new CChangeTrack;
	int len = strlen(pName)+1;
	pCur->m_pName = new char[len];
	Q_strncpy( pCur->m_pName, pName, len );
	pCur->m_nChanged = pCur->m_nUnchanged = 0;
	
	g_Tracks.AddToTail( pCur );
	
	return pCur;
}


void PrintChangeTracks()
{
	ConMsg( "\n\n" );
	ConMsg( "------------------------------------------------------------------------\n" );
	ConMsg( "CalcDelta MS / %% time / Encode MS / # Changed / # Unchanged / Class Name\n" );
	ConMsg( "------------------------------------------------------------------------\n" );

	CCycleCount total, encodeTotal;
	FOR_EACH_LL( g_Tracks, i )
	{
		CChangeTrack *pCur = g_Tracks[i];
		CCycleCount::Add( pCur->m_Count, total, total );
		CCycleCount::Add( pCur->m_EncodeCount, encodeTotal, encodeTotal );
	}

	FOR_EACH_LL( g_Tracks, j )
	{
		CChangeTrack *pCur = g_Tracks[j];
	
		ConMsg( "%6.2fms       %5.2f    %6.2fms    %4d        %4d          %s\n", 
			pCur->m_Count.GetMillisecondsF(),
			pCur->m_Count.GetMillisecondsF() * 100.0f / total.GetMillisecondsF(),
			pCur->m_EncodeCount.GetMillisecondsF(),
			pCur->m_nChanged, 
			pCur->m_nUnchanged, 
			pCur->m_pName
			);
	}

	ConMsg( "\n\n" );
	ConMsg( "Total CalcDelta MS: %.2f\n\n", total.GetMillisecondsF() );
	ConMsg( "Total Encode    MS: %.2f\n\n", encodeTotal.GetMillisecondsF() );
}


//-----------------------------------------------------------------------------
// Purpose: Entity wasn't dealt with in packet, but it has been deleted, we'll flag
//  the entity for destruction
// Input  : type - 
//			entnum - 
//			*from - 
//			*to - 
// Output : Returns true on success, false on failure.
//-----------------------------------------------------------------------------
static inline bool SV_NeedsExplicitDestroy( int entnum, CFrameSnapshot *from, CFrameSnapshot *to )
{
	// Never on uncompressed packet

	if( entnum >= to->m_nNumEntities || to->m_pEntities[entnum].m_pClass == NULL ) // doesn't exits in new
	{
		if ( entnum >= from->m_nNumEntities )
			return false; // didn't exist in old

		// in old, but not in new, destroy.
		if( from->m_pEntities[ entnum ].m_pClass != NULL ) 
		{
			return true;
		}
	}

	return false;
}


//-----------------------------------------------------------------------------
// Purpose: Creates a delta header for the entity
//-----------------------------------------------------------------------------
static inline void SV_UpdateHeaderDelta( 
	CEntityWriteInfo &u,
	int entnum )
{
	// Profiling info
	//	u.m_nTotalGap += entnum - u.m_nHeaderBase;
	//	u.m_nTotalGapCount++;

	// Keep track of number of headers so we can tell the client
	u.m_nHeaderCount++;
	u.m_nHeaderBase = entnum;
}


//
// Write the delta header. Also update the header delta info if bUpdateHeaderDelta is true.
//
// There are some cases where you want to tenatively write a header, then possibly back it out.
// In these cases:
// - pass in false for bUpdateHeaderDelta
// - store the return value from SV_WriteDeltaHeader
// - call SV_UpdateHeaderDelta ONLY if you want to keep the delta header it wrote
//
static inline void SV_WriteDeltaHeader(
	CEntityWriteInfo &u,
	int entnum,
	int flags )
{
	bf_write *pBuf = u.m_pBuf;

	// int startbit = pBuf->GetNumBitsWritten();

	int offset = entnum - u.m_nHeaderBase - 1;

	Assert ( offset >= 0 );

	SyncTag_Write( u.m_pBuf, "Hdr" );

#if defined( SOURCE_RUST_ENGINE )
	if ( s_bRustDeltaHeaderAvailable )
	{
		uint8_t headerBytes[SOURCE_MAX_DELTA_HEADER_BYTES] = {};
		uint32_t headerBits = 0;
		const SourceAbiStatus status = source_rust_bridge_delta_header_encode(
			static_cast<uint32_t>( entnum ), u.m_nHeaderBase,
			( flags & FHDR_LEAVEPVS ) != 0,
			( flags & FHDR_LEAVEPVS ) != 0 && ( flags & FHDR_DELETE ) != 0,
			( flags & FHDR_LEAVEPVS ) == 0 && ( flags & FHDR_ENTERPVS ) != 0,
			headerBytes, sizeof( headerBytes ), &headerBits );
		if ( status == SOURCE_ABI_OK && headerBits > 0 )
		{
			// Rust packs the bits in wire order, so they append unchanged.
			pBuf->WriteBits( headerBytes, static_cast<int>( headerBits ) );
			if ( !s_bRustDeltaHeaderMarkerPrinted )
			{
				ConMsg( "Rust delta header encoding active: entity %d, %u bits\n",
					entnum, headerBits );
				s_bRustDeltaHeaderMarkerPrinted = true;
			}
			SV_UpdateHeaderDelta( u, entnum );
			return;
		}

		s_bRustDeltaHeaderAvailable = false;
		ConMsg( "Rust delta header encoding disabled: entity %d, status %d\n", entnum,
			status );
	}
#endif

	pBuf->WriteUBitVar( offset );

	if ( flags & FHDR_LEAVEPVS )
	{
		pBuf->WriteOneBit( 1 ); // leave PVS bit
		pBuf->WriteOneBit( flags & FHDR_DELETE );
	}
	else
	{
		pBuf->WriteOneBit( 0 ); // delta or enter PVS
		pBuf->WriteOneBit( flags & FHDR_ENTERPVS );
	}
	
	SV_UpdateHeaderDelta( u, entnum );
}


// Calculates the delta between the two states and writes the delta and the new properties
// into u.m_pBuf. Returns false if the states are the same.
//
// Also uses the IFrameChangeList in pTo to come up with a smaller set of properties to delta against.
// It deltas against any properties that have changed since iFromFrame.
// If iFromFrame is -1, then it deltas all properties.
static int SV_CalcDeltaAndWriteProps( 
	CEntityWriteInfo &u, 
	
	const void *pFromData,
	int nFromBits, 

	PackedEntity *pTo
	)
{
	// Calculate the delta props.
	int deltaProps[MAX_DATATABLE_PROPS];
	void *pToData = pTo->GetData();
	int nToBits = pTo->GetNumBits();
	SendTable *pToTable = pTo->m_pServerClass->m_pTable;

	// TODO if our baseline is compressed, uncompress first
	Assert( !pTo->IsCompressed() );

	int nDeltaProps = SendTable_CalcDelta(
		pToTable, 
		
		pFromData,
		nFromBits,
		
		pToData,
		nToBits,
		
		deltaProps,
		ARRAYSIZE( deltaProps ),
		
		pTo->m_nEntityIndex	);

	

	// Cull out props given what the proxies say.
	int culledProps[MAX_DATATABLE_PROPS];
	
	int nCulledProps = 0;
	if ( nDeltaProps )
	{
		nCulledProps = SendTable_CullPropsFromProxies(
			pToTable,
			deltaProps, 
			nDeltaProps,
			u.m_nClientEntity-1,			
			NULL,
			-1,

			pTo->GetRecipients(),
			pTo->GetNumRecipients(),

			culledProps,
			ARRAYSIZE( culledProps ) );
	}

	
	// Write the properties.
	SendTable_WritePropList( 
		pToTable,
		
		pToData,				// object data
		pTo->GetNumBits(),

		u.m_pBuf,				// output buffer

		pTo->m_nEntityIndex,
		culledProps,
		nCulledProps );

	return nCulledProps;
}


// NOTE: to optimize this, it could store the bit offsets of each property in the packed entity.
// It would only have to store the offsets for the entities for each frame, since it only reaches 
// into the current frame's entities here.
static inline void SV_WritePropsFromPackedEntity( 
	CEntityWriteInfo &u, 
	const int *pCheckProps,
	const int nCheckProps
	)
{
	PackedEntity * pTo = u.m_pNewPack;
	PackedEntity * pFrom = u.m_pOldPack;
	SendTable *pSendTable = pTo->m_pServerClass->m_pTable;

	CServerDTITimer timer( pSendTable, SERVERDTI_WRITE_DELTA_PROPS );
	if ( g_bServerDTIEnabled && !u.m_pServer->IsHLTV() && !u.m_pServer->IsReplay() )
	{
		ICollideable *pEnt = sv.edicts[pTo->m_nEntityIndex].GetCollideable();
		ICollideable *pClientEnt = sv.edicts[u.m_nClientEntity].GetCollideable();
		if ( pEnt && pClientEnt )
		{
			float flDist = (pEnt->GetCollisionOrigin() - pClientEnt->GetCollisionOrigin()).Length();
			ServerDTI_AddEntityEncodeEvent( pSendTable, flDist );
		}
	}

	const void *pToData;
	int nToBits;

	if ( pTo->IsCompressed() )
	{
		// let server uncompress PackedEntity
		pToData = u.m_pServer->UncompressPackedEntity( pTo, nToBits );
	}
	else
	{
		// get raw data direct
		pToData = pTo->GetData();
		nToBits = pTo->GetNumBits();
	}

	Assert( pToData != NULL );

	// Cull out the properties that their proxies said not to send to this client.
	int pSendProps[MAX_DATATABLE_PROPS];
	const int *sendProps = pCheckProps;
	int nSendProps = nCheckProps;
	bf_write bufStart;


	// cull properties that are removed by SendProxies for this client.
	// don't do that for HLTV relay proxies
	if ( u.m_bCullProps )
	{
		sendProps = pSendProps;

		nSendProps = SendTable_CullPropsFromProxies( 
		pSendTable, 
		pCheckProps, 
		nCheckProps, 
		u.m_nClientEntity-1,
		
		pFrom->GetRecipients(),
		pFrom->GetNumRecipients(),
		
		pTo->GetRecipients(),
		pTo->GetNumRecipients(),

		pSendProps, 
		ARRAYSIZE( pSendProps )
		);
	}
	else
	{
		// this is a HLTV relay proxy
		bufStart = *u.m_pBuf;
	}
		
	SendTable_WritePropList(
		pSendTable, 
		pToData,
		nToBits,
		u.m_pBuf, 
		pTo->m_nEntityIndex,
		
		sendProps,
		nSendProps
		);

	if ( !u.m_bCullProps && hltv )
	{
		// this is a HLTV relay proxy, cache delta bits
		int nBits = u.m_pBuf->GetNumBitsWritten() - bufStart.GetNumBitsWritten();
		hltv->m_DeltaCache.AddDeltaBits( pTo->m_nEntityIndex, u.m_pFromSnapshot->m_nTickCount, nBits, &bufStart );
	}
}


//-----------------------------------------------------------------------------
// Purpose: See if the entity needs a "hard" reset ( i.e., and explicit creation tag )
//  This should only occur if the entity slot deleted and re-created an entity faster than
//  the last two updates toa  player.  Should never or almost never occur.  You never know though.
// Input  : type - 
//			entnum - 
//			*from - 
//			*to - 
// Output : Returns true on success, false on failure.
//-----------------------------------------------------------------------------
#if defined( SOURCE_RUST_ENGINE )
static bool SV_NeedsExplicitCreate( CEntityWriteInfo &u, CRustEntityDelta &rustDelta )
#else
static bool SV_NeedsExplicitCreate( CEntityWriteInfo &u )
#endif
{
	// Never on uncompressed packet
	if ( !u.m_bAsDelta )
	{
		return false;
	}

	const int index = u.m_nNewEntity;

#if defined( SOURCE_RUST_ENGINE )
	if ( const SourceAbiSnapshotDelta *pRustEntry = rustDelta.Find( index ) )
	{
		// Rust already compared the two snapshot identities at this index, so
		// its recreate decision is the authority here.
		if ( pRustEntry->kind == SOURCE_SNAPSHOT_DELTA_LEAVE_PVS )
		{
			// The writer only asks about an index present in both sides, so a
			// removal here means the two views disagree.
			rustDelta.Disable();
			s_bRustEntityDeltaAvailable = false;
			ConMsg( "Rust entity delta classification disabled: entity %d classified as a removal\n",
				index );
		}
		else
		{
			return pRustEntry->recreated != 0;
		}
	}
#endif

	if ( index >= u.m_pFromSnapshot->m_nNumEntities )
		return true; // entity didn't exist in old frame, so create

	// Server thinks the entity was continues, but the serial # changed, so we might need to destroy and recreate it
	const CFrameSnapshotEntry *pFromEnt = &u.m_pFromSnapshot->m_pEntities[index];
	const CFrameSnapshotEntry *pToEnt = &u.m_pToSnapshot->m_pEntities[index];

	bool bNeedsExplicitCreate = (pFromEnt->m_pClass == NULL) || pFromEnt->m_nSerialNumber != pToEnt->m_nSerialNumber;

#ifdef _DEBUG
	if ( !bNeedsExplicitCreate )
	{
		// If it doesn't need explicit create, then the classnames should match.
		// This assert is analagous to the "Server / Client mismatch" one on the client.
		static int nWhines = 0;
		if ( pFromEnt->m_pClass->GetName() != pToEnt->m_pClass->GetName() )
		{
			if ( ++nWhines < 4 )
			{
				Msg( "ERROR in SV_NeedsExplicitCreate: ent %d from/to classname (%s/%s) mismatch.\n", u.m_nNewEntity, pFromEnt->m_pClass->GetName(), pToEnt->m_pClass->GetName() );
			}
		}
	}
#endif

	return bNeedsExplicitCreate;
}


#if defined( SOURCE_RUST_ENGINE )
// Confirms that Rust reached the same enter/remove conclusion as the cursor
// walk. A disagreement means the two views of this frame have diverged, so the
// native classification takes over for the rest of the session.
static inline void SV_ConfirmRustUpdateType( CRustEntityDelta &rustDelta, int nEntityIndex,
	uint32 expectedKind )
{
	const SourceAbiSnapshotDelta *pRustEntry = rustDelta.Find( nEntityIndex );
	if ( pRustEntry != NULL && pRustEntry->kind == expectedKind )
		return;

	rustDelta.Disable();
	s_bRustEntityDeltaAvailable = false;
	ConMsg( "Rust entity delta classification disabled: entity %d expected kind %u, got %d\n",
		nEntityIndex, expectedKind, pRustEntry ? static_cast<int>( pRustEntry->kind ) : -1 );
}

static inline void SV_DetermineUpdateType( CEntityWriteInfo &u, CRustEntityDelta &rustDelta )
#else
static inline void SV_DetermineUpdateType( CEntityWriteInfo &u )
#endif
{
	// Figure out how we want to update the entity.
	if( u.m_nNewEntity < u.m_nOldEntity )
	{
		// If the entity was not in the old packet (oldnum == 9999), then 
		// delta from the baseline since this is a new entity.
		u.m_UpdateType = EnterPVS;
#if defined( SOURCE_RUST_ENGINE )
		if ( rustDelta.IsActive() )
		{
			SV_ConfirmRustUpdateType( rustDelta, u.m_nNewEntity,
				SOURCE_SNAPSHOT_DELTA_ENTER_PVS );
		}
#endif
		return;
	}
	
	if( u.m_nNewEntity > u.m_nOldEntity )
	{
		// If the entity was in the old list, but is not in the new list 
		// (newnum == 9999), then construct a special remove message.
		u.m_UpdateType = LeavePVS;
#if defined( SOURCE_RUST_ENGINE )
		if ( rustDelta.IsActive() )
		{
			SV_ConfirmRustUpdateType( rustDelta, u.m_nOldEntity,
				SOURCE_SNAPSHOT_DELTA_LEAVE_PVS );
		}
#endif
		return;
	}
	
	Assert( u.m_pToSnapshot->m_pEntities[ u.m_nNewEntity ].m_pClass );

#if defined( SOURCE_RUST_ENGINE )
	bool recreate = SV_NeedsExplicitCreate( u, rustDelta );
#else
	bool recreate = SV_NeedsExplicitCreate( u );
#endif
	
	if ( recreate )
	{
		u.m_UpdateType = EnterPVS;
		return;
	}

	// These should be the same! If they're not, then it should detect an explicit create message.
	Assert( u.m_pOldPack->m_pServerClass == u.m_pNewPack->m_pServerClass);
	
	// We can early out with the delta bits if we are using the same pack handles...
	if ( u.m_pOldPack == u.m_pNewPack )
	{
		Assert( u.m_pOldPack != NULL );
		u.m_UpdateType = PreserveEnt;
		return;
	}

#ifndef _X360
		int nBits;
#if defined( REPLAY_ENABLED )
	if ( !u.m_bCullProps && (hltv || replay) )
	{
		unsigned char *pBuffer = hltv ? hltv  ->m_DeltaCache.FindDeltaBits( u.m_nNewEntity, u.m_pFromSnapshot->m_nTickCount, nBits )
									  : replay->m_DeltaCache.FindDeltaBits( u.m_nNewEntity, u.m_pFromSnapshot->m_nTickCount, nBits );
#else
	if ( !u.m_bCullProps && hltv )
	{
		unsigned char *pBuffer = hltv->m_DeltaCache.FindDeltaBits( u.m_nNewEntity, u.m_pFromSnapshot->m_nTickCount, nBits );
#endif

		if ( pBuffer )
		{
			if ( nBits > 0 )
			{
				// Write a header.
				SV_WriteDeltaHeader( u, u.m_nNewEntity, FHDR_ZERO );

				// just write the cached bit stream 
				u.m_pBuf->WriteBits( pBuffer, nBits );

				u.m_UpdateType = DeltaEnt;
			}
			else
			{
				u.m_UpdateType = PreserveEnt;
			}

			return; // we used the cache, great
		}
	}
#endif

	int checkProps[MAX_DATATABLE_PROPS];
	int nCheckProps = u.m_pNewPack->GetPropsChangedAfterTick( u.m_pFromSnapshot->m_nTickCount, checkProps, ARRAYSIZE( checkProps ) );
	
	if ( nCheckProps == -1 )
	{
		// check failed, we have to recalc delta props based on from & to snapshot
		// that should happen only in HLTV/Replay demo playback mode, this code is really expensive

		const void *pOldData, *pNewData;
		int nOldBits, nNewBits;

		if ( u.m_pOldPack->IsCompressed() )
		{
			pOldData = u.m_pServer->UncompressPackedEntity( u.m_pOldPack, nOldBits );
		}
		else
		{
			pOldData = u.m_pOldPack->GetData();
			nOldBits = u.m_pOldPack->GetNumBits();
		}

		if ( u.m_pNewPack->IsCompressed() )
		{
			pNewData = u.m_pServer->UncompressPackedEntity( u.m_pNewPack, nNewBits );
		}
		else
		{
			pNewData = u.m_pNewPack->GetData();
			nNewBits = u.m_pNewPack->GetNumBits();
		}

		nCheckProps = SendTable_CalcDelta(
			u.m_pOldPack->m_pServerClass->m_pTable, 
			pOldData,
			nOldBits,
			pNewData,
			nNewBits,
			checkProps,
			ARRAYSIZE( checkProps ),
			u.m_nNewEntity
			);
	}

#ifndef NO_VCR
	if ( vcr_verbose.GetInt() )
	{
		VCRGenericValueVerify( "checkProps", checkProps, sizeof( checkProps[0] ) * nCheckProps );
	}
#endif
	
	if ( nCheckProps > 0 )
	{
		// Write a header.
		SV_WriteDeltaHeader( u, u.m_nNewEntity, FHDR_ZERO );
#if defined( DEBUG_NETWORKING )
		int startBit = u.m_pBuf->GetNumBitsWritten();
#endif
		SV_WritePropsFromPackedEntity( u, checkProps, nCheckProps );
#if defined( DEBUG_NETWORKING )
		int endBit = u.m_pBuf->GetNumBitsWritten();
		TRACE_PACKET( ( "    Delta Bits (%d) = %d (%d bytes)\n", u.m_nNewEntity, (endBit - startBit), ( (endBit - startBit) + 7 ) / 8 ) );
#endif
		// If the numbers are the same, then the entity was in the old and new packet.
		// Just delta compress the differences.
		u.m_UpdateType = DeltaEnt;
	}
	else
	{
#ifndef _X360
		if ( !u.m_bCullProps )
		{
			if ( hltv )
			{
				// no bits changed, PreserveEnt
				hltv->m_DeltaCache.AddDeltaBits( u.m_nNewEntity, u.m_pFromSnapshot->m_nTickCount, 0, NULL );
			}

#if defined( REPLAY_ENABLED )
			if ( replay )
			{
				// no bits changed, PreserveEnt
				replay->m_DeltaCache.AddDeltaBits( u.m_nNewEntity, u.m_pFromSnapshot->m_nTickCount, 0, NULL );
			}
#endif
		}
#endif
		u.m_UpdateType = PreserveEnt;
	}
}

static inline ServerClass* GetEntServerClass(edict_t *pEdict)
{
	return pEdict->GetNetworkable()->GetServerClass();
}



static inline void SV_WriteEnterPVS( CEntityWriteInfo &u )
{
	TRACE_PACKET(( "  SV Enter PVS (%d) %s\n", u.m_nNewEntity, u.m_pNewPack->m_pServerClass->m_pNetworkName ) );

	SV_WriteDeltaHeader( u, u.m_nNewEntity, FHDR_ENTERPVS );

	Assert( u.m_nNewEntity < u.m_pToSnapshot->m_nNumEntities );

	CFrameSnapshotEntry *entry = &u.m_pToSnapshot->m_pEntities[u.m_nNewEntity];

	ServerClass *pClass = entry->m_pClass;

	if ( !pClass )
	{
		Host_Error("SV_CreatePacketEntities: GetEntServerClass failed for ent %d.\n", u.m_nNewEntity);
	}
	
	TRACE_PACKET(( "  SV Enter Class %s\n", pClass->m_pNetworkName ) );

	if ( pClass->m_ClassID >= u.m_pServer->serverclasses )
	{
		ConMsg( "pClass->m_ClassID(%i) >= %i\n", pClass->m_ClassID, u.m_pServer->serverclasses );
		Assert( 0 );
	}

	u.m_pBuf->WriteUBitLong( pClass->m_ClassID, u.m_pServer->serverclassbits );
	
	// Write some of the serial number's bits. 
	u.m_pBuf->WriteUBitLong( entry->m_nSerialNumber, NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS );

	// Get the baseline.
	// Since the ent is in the fullpack, then it must have either a static or an instance baseline.
	PackedEntity *pBaseline = u.m_bAsDelta ? framesnapshotmanager->GetPackedEntity( u.m_pBaseline, u.m_nNewEntity ) : NULL;
	const void *pFromData;
	int nFromBits;

	if ( pBaseline && (pBaseline->m_pServerClass == u.m_pNewPack->m_pServerClass) )
	{
		Assert( !pBaseline->IsCompressed() );
		pFromData = pBaseline->GetData();
		nFromBits = pBaseline->GetNumBits();
	}
	else
	{
		// Since the ent is in the fullpack, then it must have either a static or an instance baseline.
		int nFromBytes;
		if ( !u.m_pServer->GetClassBaseline( pClass, &pFromData, &nFromBytes ) )
		{
			Error( "SV_WriteEnterPVS: missing instance baseline for '%s'.", pClass->m_pNetworkName );
		}

		ErrorIfNot( pFromData,
			("SV_WriteEnterPVS: missing pFromData for '%s'.", pClass->m_pNetworkName)
		);
		
		nFromBits = nFromBytes * 8;	// NOTE: this isn't the EXACT number of bits but that's ok since it's
									// only used to detect if we overran the buffer (and if we do, it's probably
									// by more than 7 bits).
	}

	if ( u.m_pTo->from_baseline )
	{
		// remember that we sent this entity as full update from entity baseline
		u.m_pTo->from_baseline->Set( u.m_nNewEntity );
	}

	const void *pToData;
	int nToBits;

	if ( u.m_pNewPack->IsCompressed() )
	{
		pToData = u.m_pServer->UncompressPackedEntity( u.m_pNewPack, nToBits );
	}
	else
	{
		pToData = u.m_pNewPack->GetData();
		nToBits = u.m_pNewPack->GetNumBits();
	}

	/*if ( server->IsHLTV() || server->IsReplay() )
	{*/
	// send all changed properties when entering PVS (no SendProxy culling since we may use it as baseline
	u.m_nFullProps +=  SendTable_WriteAllDeltaProps( pClass->m_pTable, pFromData, nFromBits,
		pToData, nToBits, u.m_pNewPack->m_nEntityIndex, u.m_pBuf );
	/*}
	else
	{
		// remove all props that are excluded for this client
		u.m_nFullProps += SV_CalcDeltaAndWriteProps( u, pFromData, nFromBits, u.m_pNewPack );
	}*/

	if ( u.m_nNewEntity == u.m_nOldEntity )
		u.NextOldEntity();  // this was a entity recreate

	u.NextNewEntity();
}


static inline void SV_WriteLeavePVS( CEntityWriteInfo &u )
{
	int headerflags = FHDR_LEAVEPVS;
	bool deleteentity = false;
	
	if ( u.m_bAsDelta )
	{
		deleteentity = SV_NeedsExplicitDestroy( u.m_nOldEntity, u.m_pFromSnapshot, u.m_pToSnapshot );	
	}
	
	if ( deleteentity )
	{
		// Mark that we handled deletion of this index
		u.m_DeletionFlags.Set( u.m_nOldEntity );

		headerflags |= FHDR_DELETE;
	}

	TRACE_PACKET( ( "  SV Leave PVS (%d) %s %s\n", u.m_nOldEntity, 
		deleteentity ? "deleted" : "left pvs",
		u.m_pOldPack->m_pServerClass->m_pNetworkName ) );

	SV_WriteDeltaHeader( u, u.m_nOldEntity, headerflags );

	u.NextOldEntity();
}


static inline void SV_WriteDeltaEnt( CEntityWriteInfo &u )
{
	TRACE_PACKET( ( "  SV Delta PVS (%d %d) %s\n", u.m_nNewEntity, u.m_nOldEntity, u.m_pOldPack->m_pServerClass->m_pNetworkName ) );

	// NOTE: it was already written in DetermineUpdateType. By doing it this way, we avoid an expensive
	// (non-byte-aligned) copy of the data.

	u.NextOldEntity();
	u.NextNewEntity();
}


static inline void SV_PreserveEnt( CEntityWriteInfo &u )
{
	TRACE_PACKET( ( "  SV Preserve PVS (%d) %s\n", u.m_nOldEntity, u.m_pOldPack->m_pServerClass->m_pNetworkName ) );

	// updateType is preserveEnt. The client will detect this because our next entity will have a newnum
	// that is greater than oldnum, in which case the client just keeps the current entity alive.
	u.NextOldEntity();
	u.NextNewEntity();
}


static inline void SV_WriteEntityUpdate( CEntityWriteInfo &u )
{
	switch( u.m_UpdateType )
	{
		case EnterPVS:
		{
			SV_WriteEnterPVS( u );
		}
		break;

		case LeavePVS:
		{
			SV_WriteLeavePVS( u );
		}
		break;

		case DeltaEnt:
		{
			SV_WriteDeltaEnt( u );
		}
		break;

		case PreserveEnt:
		{
			SV_PreserveEnt( u );
		}
		break;
	}
}


static inline int SV_WriteDeletions( CEntityWriteInfo &u )
{
	if( !u.m_bAsDelta )
		return 0;

	int nNumDeletions = 0;

	CFrameSnapshot *pFromSnapShot = u.m_pFromSnapshot;
	CFrameSnapshot *pToSnapShot = u.m_pToSnapshot;

	int nLast = MAX( pFromSnapShot->m_nNumEntities, pToSnapShot->m_nNumEntities );
	for ( int i = 0; i < nLast; i++ )
	{
		// Packet update didn't clear it out expressly
		if ( u.m_DeletionFlags.Get( i ) ) 
			continue;

		// If the entity is marked to transmit in the u.m_pTo, then it can never be destroyed by the m_iExplicitDeleteSlots
		// Another possible fix would be to clear any slots in the explicit deletes list that were actually occupied when a snapshot was taken
		if ( u.m_pTo->transmit_entity.Get(i) )
			continue;

		// Looks like it should be gone
		bool bNeedsExplicitDelete = SV_NeedsExplicitDestroy( i, pFromSnapShot, pToSnapShot );
		if ( !bNeedsExplicitDelete && u.m_pTo )
		{
			bNeedsExplicitDelete = ( pToSnapShot->m_iExplicitDeleteSlots.Find(i) != pToSnapShot->m_iExplicitDeleteSlots.InvalidIndex() );
			// We used to do more stuff here as a sanity check, but I don't think it was necessary since the only thing that would unset the bould would be a "recreate" in the same slot which is
			// already implied by the u.m_pTo->transmit_entity.Get(i) check
		}

		// Check conditions
		if ( bNeedsExplicitDelete )
		{
			TRACE_PACKET( ( "  SV Explicit Destroy (%d)\n", i ) );

			u.m_pBuf->WriteOneBit(1);
			u.m_pBuf->WriteUBitLong( i, MAX_EDICT_BITS );
			++nNumDeletions;
		}
	}
	// No more entities..
	u.m_pBuf->WriteOneBit(0); 

	return nNumDeletions;
}


/*
=============
WritePacketEntities

Computes either a compressed, or uncompressed delta buffer for the client.
Returns the size IN BITS of the message buffer created.
=============
*/

void CBaseServer::WriteDeltaEntities( CBaseClient *client, CClientFrame *to, CClientFrame *from, bf_write &pBuf )
{
	VPROF_BUDGET( "CBaseServer::WriteDeltaEntities", VPROF_BUDGETGROUP_OTHER_NETWORKING );
	// Setup the CEntityWriteInfo structure.
	CEntityWriteInfo u;
	u.m_pBuf = &pBuf;
	u.m_pTo = to;
	u.m_pToSnapshot = to->GetSnapshot();
	u.m_pBaseline = client->m_pBaseline;
	u.m_nFullProps = 0;
	u.m_pServer = this;
	u.m_nClientEntity = client->m_nEntityIndex;
#ifndef _XBOX
	if ( IsHLTV() || IsReplay() )
	{
		// cull props only on master proxy
		u.m_bCullProps = sv.IsActive();
	}
	else
#endif
	{
		u.m_bCullProps = true;	// always cull props for players
	}
	
	if ( from != NULL )
	{
		u.m_bAsDelta = true;	
		u.m_pFrom = from;
		u.m_pFromSnapshot = from->GetSnapshot();
		Assert( u.m_pFromSnapshot );
	}
	else
	{
		u.m_bAsDelta = false;
		u.m_pFrom = NULL;
		u.m_pFromSnapshot = NULL;
	}

	u.m_nHeaderCount = 0;
//	u.m_nTotalGap = 0;
//	u.m_nTotalGapCount = 0;

	// set from_baseline pointer if this snapshot may become a baseline update
	if ( client->m_nBaselineUpdateTick == -1 )
	{
		client->m_BaselinesSent.ClearAll();
		to->from_baseline = &client->m_BaselinesSent;
	}

	// Write the header, TODO use class SVC_PacketEntities
		
	TRACE_PACKET(( "WriteDeltaEntities (%d)\n", u.m_pToSnapshot->m_nNumEntities ));

	u.m_pBuf->WriteUBitLong( svc_PacketEntities, NETMSG_TYPE_BITS );

	u.m_pBuf->WriteUBitLong( u.m_pToSnapshot->m_nNumEntities, MAX_EDICT_BITS );
	
	if ( u.m_bAsDelta )
	{
		u.m_pBuf->WriteOneBit( 1 ); // use delta sequence

		u.m_pBuf->WriteLong( u.m_pFrom->tick_count );    // This is the sequence # that we are updating from.
	}
	else
	{
		u.m_pBuf->WriteOneBit( 0 );	// use baseline
	}

	u.m_pBuf->WriteUBitLong ( client->m_nBaselineUsed, 1 );	// tell client what baseline we are using

	// Store off current position 
	bf_write savepos = *u.m_pBuf;

	// Save room for number of headers to parse, too
	u.m_pBuf->WriteUBitLong ( 0, MAX_EDICT_BITS+DELTASIZE_BITS+1 );	
		
	int startbit = u.m_pBuf->GetNumBitsWritten();

	bool bIsTracing = client->IsTracing();
	if ( bIsTracing )
	{
		client->TraceNetworkData( pBuf, "Delta Entities Overhead" );
	}

	// Don't work too hard if we're using the optimized single-player mode.
	if ( !g_pLocalNetworkBackdoor )
	{
#if defined( SOURCE_RUST_ENGINE )
		// Classify this receiver's whole delta before writing any of it, so
		// each entity decision reads an already-agreed answer.
		CRustEntityDelta rustDelta;
		rustDelta.Build( u );
#endif

		// Iterate through the in PVS bitfields until we find an entity 
		// that was either in the old pack or the new pack
		u.NextOldEntity();
		u.NextNewEntity();
		
		while ( (u.m_nOldEntity != ENTITY_SENTINEL) || (u.m_nNewEntity != ENTITY_SENTINEL) )
		{
			u.m_pNewPack = (u.m_nNewEntity != ENTITY_SENTINEL) ? framesnapshotmanager->GetPackedEntity( u.m_pToSnapshot, u.m_nNewEntity ) : NULL;
			u.m_pOldPack = (u.m_nOldEntity != ENTITY_SENTINEL) ? framesnapshotmanager->GetPackedEntity( u.m_pFromSnapshot, u.m_nOldEntity ) : NULL;
			int nEntityStartBit = pBuf.GetNumBitsWritten();

			// Figure out how we want to write this entity.
#if defined( SOURCE_RUST_ENGINE )
			SV_DetermineUpdateType( u, rustDelta );
#else
			SV_DetermineUpdateType( u  );
#endif
			SV_WriteEntityUpdate( u );

			if ( !bIsTracing )
				continue;

			switch ( u.m_UpdateType )
			{
			default:
			case PreserveEnt:
				break;
			case EnterPVS:
				{
					char const *eString = sv.edicts[ u.m_pNewPack->m_nEntityIndex ].GetNetworkable()->GetClassName();
					client->TraceNetworkData( pBuf, "enter [%s]", eString );
					ETWMark1I( eString, pBuf.GetNumBitsWritten() - nEntityStartBit );
				}
				break;
			case LeavePVS:
				{
					// Note, can't use GetNetworkable() since the edict has been freed at this point
					char const *eString = u.m_pOldPack->m_pServerClass->m_pNetworkName;
					client->TraceNetworkData( pBuf, "leave [%s]", eString );
					ETWMark1I( eString, pBuf.GetNumBitsWritten() - nEntityStartBit );
				}
				break;
			case DeltaEnt:
				{
					char const *eString = sv.edicts[ u.m_pOldPack->m_nEntityIndex ].GetNetworkable()->GetClassName();
					client->TraceNetworkData( pBuf, "delta [%s]", eString );
					ETWMark1I( eString, pBuf.GetNumBitsWritten() - nEntityStartBit );
				}
				break;
			}
		}

		// Now write out the express deletions
		int nNumDeletions = SV_WriteDeletions( u );
		if ( bIsTracing )
		{
			client->TraceNetworkData( pBuf, "Delta: [%d] deletions", nNumDeletions );
		}
	}

	// get number of written bits
	int length = u.m_pBuf->GetNumBitsWritten() - startbit;

	// go back to header and fill in correct length now
	savepos.WriteUBitLong( u.m_nHeaderCount, MAX_EDICT_BITS );
	savepos.WriteUBitLong( length, DELTASIZE_BITS );

	bool bUpdateBaseline = ( (client->m_nBaselineUpdateTick == -1) && 
		(u.m_nFullProps > 0 || !u.m_bAsDelta) );

	if ( bUpdateBaseline && u.m_pBaseline )
	{
		// tell client to use this snapshot as baseline update
		savepos.WriteOneBit( 1 ); 
		client->m_nBaselineUpdateTick = to->tick_count;
	}
	else
	{
		savepos.WriteOneBit( 0 ); 
	}

	if ( bIsTracing )
	{
		client->TraceNetworkData( pBuf, "Delta Finish" );
	}
}





