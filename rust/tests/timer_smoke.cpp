#include "tier0/fasttimer.h"
#include "tier0/platform.h"

#include <chrono>
#include <cmath>
#include <cstdio>
#include <thread>

int main()
{
	CFastTimer timer;
	const double start = Plat_FloatTime();
	timer.Start();
	std::this_thread::sleep_for(std::chrono::milliseconds(50));
	timer.End();
	const double monotonicSeconds = Plat_FloatTime() - start;
	const double fastTimerSeconds = timer.GetDuration().GetSeconds();

	std::printf("monotonic=%.6f fasttimer=%.6f clock=%lld\n",
		monotonicSeconds, fastTimerSeconds,
		static_cast<long long>(CFastTimer::GetClockSpeed()));
	if (monotonicSeconds < 0.045 || fastTimerSeconds < 0.045)
		return 1;
	if (std::fabs(monotonicSeconds - fastTimerSeconds) >= 0.010)
		return 2;
#if (defined(__arm__) || defined(__aarch64__)) && defined(POSIX)
	if (CFastTimer::GetClockSpeed() != 1000000000LL)
		return 3;
#endif
	return 0;
}

