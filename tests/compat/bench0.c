/* bench0: the cost of one forwarded call (COMPAT_STAGE0_PLAN.md section 8). 10,000 write(-1, 0, 0) calls, which Linux
 * answers with EBADF and so must the bridge, timed with rdtsc; prints the average cycles per call. Not compared
 * byte for byte (timing differs); its exit code (0 when every call gave -1) is. Built as hello0. */
#include <stdio.h>
#include <unistd.h>

static unsigned long long rdtsc(void)
{
    unsigned lo, hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    return ((unsigned long long)hi << 32) | lo;
}

int main(void)
{
    int bad = 0;
    unsigned long long t0 = rdtsc();
    for (int i = 0; i < 10000; i++)
        if (write(-1, 0, 0) != -1)
            bad++;
    unsigned long long t1 = rdtsc();
    printf("cycles per call %llu\n", (t1 - t0) / 10000);
    return bad ? 1 : 0;
}
