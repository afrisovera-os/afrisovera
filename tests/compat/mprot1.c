/* mprot1 (Daraja L1.1, COMPAT_L1_PLAN 8 point 1): mprotect that ADDS rights after the start, which AfriSovera
 * refuses with EACCES while Linux allows it. Not an equality test: each side is graded against its own expectation
 * (pc-compat0 for AfriSovera, compat-linux-ref for Linux), so the difference is shown, not hidden.
 *   1. WRITE added to a read-only anonymous page;
 *   2. EXEC added to a read-write anonymous page;
 *   3. WRITE added to the program's own code page (where main lies).
 * Each line prints 0 or the errno's name. Exit 0 when all three were refused with EACCES, 1 when all three
 * succeeded (Linux), 2 for any mix.
 * Written from the Linux manual pages mmap(2) and mprotect(2), and errno(3). */
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <sys/mman.h>

static int refused, allowed;

static void try(const char *what, void *a, int prot) {
    int r = mprotect(a, 4096, prot);
    if (r == 0) {
        allowed++;
        printf("%s: 0\n", what);
    } else {
        if (errno == EACCES) refused++;
        printf("%s: %s\n", what, errno == EACCES ? "EACCES" : "other error");
    }
}

int main(void) {
    unsigned char *ro = mmap(0, 4096, PROT_READ, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    unsigned char *rw = mmap(0, 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (ro == MAP_FAILED || rw == MAP_FAILED) {
        printf("mmap failed\n");
        return 3;
    }
    try("add WRITE to a read-only page", ro, PROT_READ | PROT_WRITE);
    try("add EXEC to a read-write page", rw, PROT_READ | PROT_WRITE | PROT_EXEC);
    void *code = (void *)((uintptr_t)&main & ~(uintptr_t)4095);
    try("add WRITE to the code page", code, PROT_READ | PROT_WRITE | PROT_EXEC);
    if (refused == 3) return 0;
    if (allowed == 3) return 1;
    return 2;
}
