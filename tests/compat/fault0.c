/* fault0: Daraja stage 0's fault case (COMPAT_STAGE0_PLAN.md section 7). A write through a null pointer. On Linux the
 * shell reports status 139 (128 + SIGSEGV 11); through the bridge the FAULT message must give the same status.
 * Built as hello0. The pointer is volatile so the compiler keeps the store. */
int main(void)
{
    volatile int *p = 0;
    *p = 1;
    return 0;
}
