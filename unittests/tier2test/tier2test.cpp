//========= Copyright Valve Corporation, All rights reserved. ============//
//
// Purpose: Unit test program for testing of tier2 libraries
//
// $NoKeywords: $
//=============================================================================//

#include "unitlib/unitlib.h"
#include "filesystem.h"
#include "tier2/tier2.h"
#include "tier2/utlstreambuffer.h"
#include "mathlib/mathlib.h"
#include <stdio.h>
#include <string.h>


//-----------------------------------------------------------------------------
// Used to connect/disconnect the DLL
//-----------------------------------------------------------------------------
class CTier2TestAppSystem : public CTier2AppSystem< IAppSystem >
{
	typedef CTier2AppSystem< IAppSystem > BaseClass;

public:
	virtual bool Connect( CreateInterfaceFn factory ) 
	{
		if ( !BaseClass::Connect( factory ) )
			return false;

		if ( !g_pFullFileSystem )
			return false;
		return true; 
	}

	virtual InitReturnVal_t Init()
	{
		MathLib_Init( 2.2f, 2.2f, 0.0f, 2.0f );

		InitReturnVal_t nRetVal = BaseClass::Init();
		if ( nRetVal != INIT_OK )
			return nRetVal;

		return INIT_OK;
	}
};

USE_UNITTEST_APPSYSTEM( CTier2TestAppSystem )


DEFINE_TESTSUITE( UtlStreamBufferTestSuite )

DEFINE_TESTCASE( UtlStreamBufferSeekFlushesPendingByte, UtlStreamBufferTestSuite )
{
	const char *pFileName = "tier2_streambuffer_seek_test.bin";
	remove( pFileName );

	{
		CUtlStreamBuffer buffer( pFileName, NULL );
		Shipping_Assert( buffer.IsOpen() );

		const unsigned char payload[] = { 1, 2, 3, 4, 5 };
		buffer.Put( payload, sizeof( payload ) );
		buffer.SeekPut( CUtlBuffer::SEEK_HEAD, 0 );

		const unsigned char replacement[] = { 9, 8 };
		buffer.Put( replacement, sizeof( replacement ) );
	}

	FILE *pFile = fopen( pFileName, "rb" );
	Shipping_Assert( pFile != NULL );
	if ( pFile )
	{
		unsigned char actual[5] = {};
		Shipping_Assert( fread( actual, 1, sizeof( actual ), pFile ) == sizeof( actual ) );
		Shipping_Assert( fgetc( pFile ) == EOF );
		fclose( pFile );

		const unsigned char expected[] = { 9, 8, 3, 4, 5 };
		Shipping_Assert( memcmp( actual, expected, sizeof( expected ) ) == 0 );
	}

	remove( pFileName );
}
