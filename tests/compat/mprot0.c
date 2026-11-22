/* mprot0 (Daraja L1.1, COMPAT_L1_PLAN 6 step 1): mprotect that only REMOVES rights, the cases Linux and AfriSovera
 * answer alike, so stdout, stderr and the exit code must be byte-equal on both.
 *   1. an anonymous private mapping of 3 pages, read-write; every page written and read back;
 *   2. page 1 made read-only (write removed): 0, and its bytes still read;
 *   3. page 2 made PROT_NONE (everything removed): 0;
 *   4. pages 0 to 2 unmapped: 0.
 * Written from the Linux manual pages mmap(2), mprotect(2) and munmap(2). */
#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <sys/mman.h>

static int step(const char *what, int r) {
    printf("%s: %d%s%s\n", what, r, r ? " errno " : "", r ? strerror(errno) : "");
    return r;
}

int main(void) {
    const size_t pg = 4096;
    unsigned char *p = mmap(0, 3 * pg, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (p == MAP_FAILED) {
        printf("mmap: failed errno %s\n", strerror(errno));
        return 1;
    }
    printf("mmap: ok, page aligned %d\n", ((unsigned long)p % pg) == 0);
    for (size_t i = 0; i < 3 * pg; i++) p[i] = (unsigned char)(i * 7);
    unsigned sum = 0;
    for (size_t i = 0; i < 3 * pg; i++) sum += p[i];
    printf("written and read back: sum %u\n", sum);
    int bad = 0;
    bad |= step("page 1 read-only", mprotect(p + pg, pg, PROT_READ));
    unsigned sum1 = 0;
    for (size_t i = pg; i < 2 * pg; i++) sum1 += p[i];
    printf("page 1 still reads: sum %u\n", sum1);
    bad |= step("page 2 no access", mprotect(p + 2 * pg, pg, PROT_NONE));
    bad |= step("unmap all three", munmap(p, 3 * pg));
    return bad ? 2 : 0;
}
