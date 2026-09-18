//========= Copyright Valve Corporation, All rights reserved. ============//
//
// Purpose:
//
//=============================================================================

#include "tier0/tslist.h"
#include "tier0/fasttimer.h"
#include "tier0/platform.h"
#include "tier0/threadtools.h"
#include <list>
#include <stdlib.h>
#include <math.h>
#if defined(POSIX)
#include <unistd.h>
#endif
#if defined( _X360 )
#include "xbox/xbox_win32stubs.h"
#endif

#include "unitlib/unitlib.h"

DEFINE_TESTSUITE( FastTimerTestSuite )

DEFINE_TESTCASE( FastTimerTracksMonotonicTime, FastTimerTestSuite )
{
	Msg( "CFastTimer monotonic conversion test..." );
	CFastTimer timer;
	const double start = Plat_FloatTime();
	timer.Start();
#if defined(POSIX)
	usleep( 50000 );
#else
	ThreadSleep( 50 );
#endif
	timer.End();
	const double wallDuration = Plat_FloatTime() - start;
	const double timerDuration = timer.GetDuration().GetSeconds();

	Shipping_Assert( wallDuration >= 0.045 );
	Shipping_Assert( timerDuration >= 0.045 );
	Shipping_Assert( fabs( timerDuration - wallDuration ) < 0.010 );

#if (defined(__arm__) || defined(__aarch64__)) && defined(POSIX)
	Shipping_Assert( CFastTimer::GetClockSpeed() == 1000000000LL );
#endif
	Msg( "pass\n" );
}

DEFINE_TESTSUITE( TSListTestSuite )

DEFINE_TESTCASE( TSListTest, TSListTestSuite )
{
	RunTSListTests( 50000 );

	RunTSQueueTests( 50000 );
}
