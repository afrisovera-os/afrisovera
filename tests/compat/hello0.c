/* hello0: the first end-to-end program of Daraja stage 0 (COMPAT_STAGE0_PLAN.md section 7). Built by us from this
 * source with musl 1.2.5, static, not position-independent, linked at USER_BASE + 0x40_0000 so the same binary runs
 * on Linux and inside AfriSovera's foreign area:
 *   musl-gcc -static -no-pie -Os -Wl,-Ttext-segment=0x10000400000 -o hello0 hello0.c
 * What it does, all of it deterministic: prints a greeting through stdio (so musl's own start-up and buffered
 * output run), its arguments, then opens data/greeting.txt, reads it into a fixed buffer (no malloc: brk and mmap are
 * stage L1), writes it out with its length and a checksum, one line on stderr, and exits with 7.
 * Its output and exit code on Linux are the reference; AfriSovera must give the same bytes. */
#include <fcntl.h>
#include <stdio.h>
#include <unistd.h>

static char buf[4096];

int main(int argc, char **argv)
{
    printf("hello from a Linux program\n");
    printf("argc %d\n", argc);
    for (int i = 0; i < argc; i++)
        printf("argv[%d] %s\n", i, argv[i]);
    fflush(stdout);

    int fd = open("data/greeting.txt", O_RDONLY);
    if (fd < 0) {
        fprintf(stderr, "open failed\n");
        return 2;
    }
    ssize_t n = read(fd, buf, sizeof buf);
    close(fd);
    if (n < 0) {
        fprintf(stderr, "read failed\n");
        return 3;
    }
    unsigned sum = 0;
    for (ssize_t i = 0; i < n; i++)
        sum = sum * 31 + (unsigned char)buf[i];
    write(1, buf, (size_t)n);
    printf("read %ld bytes, checksum %08x\n", (long)n, sum);
    fprintf(stderr, "this line is on stderr\n");
    return 7;
}
