/* fault0: Daraja stage 0's fault case (COMPAT_STAGE0_PLAN.md section 7). A write through a null pointer. On Linux the
 * shell reports status 139 (128 + SIGSEGV 11); through the bridge the FAULT message must give the same status.
 * Built as hello0. The pointer is volatile so the compiler keeps the store.
 *
 * Before the fault it makes the same call 5 times: getpriority(2) (x86-64 number 140), which the bridge does not
 * answer (ENOSYS). The bridge's unknown-call limit shows 3 lines and, on the fault, its summary
 * "[daraja] unknown call 140: 2 more not shown" (the architect, 21 Nov: the summary on every exit path, a faulting
 * program first). On Linux the 5 calls simply succeed; the call order stays equal on both sides. The result is
 * not used, so nothing printed differs. Written from the Linux manual pages getpriority(2) and syscall(2). */
#define _GNU_SOURCE
#include <unistd.h>

int main(void)
{
    for (int i = 0; i < 5; i++)
        syscall(140, 0, 0); /* getpriority(PRIO_PROCESS, 0) */
    volatile int *p = 0;
    *p = 1;
    return 0;
}
