/* clock1 (Daraja, call 79; CLOCK_PLAN 4): the clocks a Linux program reads, checked by the program itself so that
 * stdout, stderr and the exit code are byte-equal on Linux and on AfriSovera; it prints no times, only verdicts.
 *   1. CLOCK_MONOTONIC around a 200 ms nanosleep: the difference lies between 150 and 2,000 ms;
 *   2. 1,000 CLOCK_MONOTONIC reads in a row: none less than the one before;
 *   3. CLOCK_REALTIME: after 2026-01-01, and within 2 s of gettimeofday's;
 *   4. CLOCK_REALTIME around the same sleep: it moved between 150 and 2,000 ms;
 *   5. clock_getres(CLOCK_MONOTONIC): more than 0 and at most 1 ms;
 *   6. CLOCK_PROCESS_CPUTIME_ID and CLOCK_BOOTTIME answer;
 *   7. clock_gettime with an id no system has (12345): -1 with EINVAL.
 * Exit 0 when all seven hold, else 1.
 * Written from the Linux manual pages clock_gettime(2), clock_getres(2), gettimeofday(2) and nanosleep(2). */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/time.h>
#include <time.h>

static long long ns(const struct timespec *t) {
    return (long long)t->tv_sec * 1000000000LL + t->tv_nsec;
}

static int verdict(const char *what, int ok) {
    printf("%s: %s\n", what, ok ? "ok" : "WRONG");
    return ok ? 0 : 1;
}

int main(void) {
    int bad = 0;
    struct timespec m0, m1, r0, r1, res, t;
    struct timespec nap = {0, 200000000};

    clock_gettime(CLOCK_MONOTONIC, &m0);
    clock_gettime(CLOCK_REALTIME, &r0);
    nanosleep(&nap, 0);
    clock_gettime(CLOCK_MONOTONIC, &m1);
    clock_gettime(CLOCK_REALTIME, &r1);
    long long dm = (ns(&m1) - ns(&m0)) / 1000000, dr = (ns(&r1) - ns(&r0)) / 1000000;
    bad |= verdict("monotonic around a 200 ms sleep", dm >= 150 && dm <= 2000);

    long long prev = ns(&m1);
    int back = 0;
    for (int i = 0; i < 1000; i++) {
        clock_gettime(CLOCK_MONOTONIC, &t);
        if (ns(&t) < prev) back++;
        prev = ns(&t);
    }
    bad |= verdict("monotonic never backwards", back == 0);

    struct timeval tv;
    gettimeofday(&tv, 0);
    long long gap = (long long)tv.tv_sec - (long long)r1.tv_sec;
    bad |= verdict("realtime after 2026 and beside gettimeofday", r1.tv_sec > 1767225600 && gap >= -2 && gap <= 2);
    bad |= verdict("realtime around the same sleep", dr >= 150 && dr <= 2000);

    bad |= verdict("getres of monotonic", clock_getres(CLOCK_MONOTONIC, &res) == 0 && ns(&res) > 0 && ns(&res) <= 1000000);
    bad |= verdict("cputime and boottime answer",
                   clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &t) == 0 && clock_gettime(CLOCK_BOOTTIME, &t) == 0);

    errno = 0;
    int r = clock_gettime((clockid_t)12345, &t);
    printf("an unknown clock: %d %s\n", r, r ? strerror(errno) : "");
    bad |= (r != -1 || errno != EINVAL);
    return bad;
}
