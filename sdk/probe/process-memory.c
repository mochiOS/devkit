#include <mochios/process.h>
#include <errno.h>
#include <stddef.h>
#include <stdint.h>
#include <sys/mman.h>

_Static_assert(sizeof(struct mochios_process_memory_usage) == 16, "memory ABI size");
_Static_assert(offsetof(struct mochios_process_memory_usage, peak_bytes) == 8, "memory ABI offset");

/* Called from an actual executable and imported through this shared object. */
int sdk_memory_probe(void)
{
    struct mochios_process_memory_usage before, after, released;
    if (mochios_process_memory_read(&before, sizeof(before)) ||
        !before.current_bytes || before.current_bytes > before.peak_bytes)
        return 1;
    errno = 0;
    if (mochios_process_memory_read(&after, sizeof(after) - 1) != -1 || errno != EINVAL)
        return 2;
    errno = 0;
    if (mochios_process_memory_read(NULL, sizeof(after)) != -1 || errno != EFAULT)
        return 3;
    const size_t length = 256 * 1024;
    volatile unsigned char *allocation = mmap(NULL, length, PROT_READ | PROT_WRITE,
        MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (allocation == MAP_FAILED)
        return 4;
    for (size_t i = 0; i < length; i += 4096)
        allocation[i] = (unsigned char)i;
    int result = mochios_process_memory_read(&after, sizeof(after));
    int unmap_result = munmap((void *)allocation, length);
    if (result || unmap_result || after.current_bytes < before.current_bytes + length ||
        after.peak_bytes < after.current_bytes)
        return 5;
    if (mochios_process_memory_read(&released, sizeof(released)) ||
        released.current_bytes >= after.current_bytes || released.peak_bytes < after.peak_bytes)
        return 6;
    return 0;
}
