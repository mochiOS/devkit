#define _GNU_SOURCE 1
#include <dlfcn.h>
#include <link.h>
#include <pthread.h>
#include <sys/file.h>
#include <sys/mman.h>
#include <unistd.h>
#include <errno.h>
#include <stdint.h>

_Static_assert(sizeof(Dl_info) == 32, "Dl_info ABI");
_Static_assert(sizeof(struct dl_phdr_info) == 32, "dl_phdr_info common prefix ABI");
_Static_assert(sizeof(Elf64_Phdr) == 56, "ELF64 header ABI");
_Static_assert(LOCK_SH == 1 && LOCK_EX == 2 && LOCK_NB == 4 && LOCK_UN == 8, "flock ABI");

static int stop(struct dl_phdr_info *info, size_t size, void *data)
{
    if (size < sizeof(*info) || !info->dlpi_name || !info->dlpi_phdr || !info->dlpi_phnum)
        return 91;
    *(int *)data = 1;
    return 17;
}

/* The SDK verifier compiles and links this against the measured export set.
   Execution is a separate native test requiring the matching OS components. */
int sdk_unix_extensions_probe(void)
{
    const size_t page = 4096;
    unsigned char *memory = mmap(0, page * 2, PROT_READ | PROT_WRITE,
                                MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    if (memory == MAP_FAILED) return 1;
    memory[0] = 71;
    memory[page] = 89;
    unsigned char resident[2] = { 0, 0 };
    int result = 0;
    if (mincore(memory, page * 2, resident) || !(resident[0] & 1) || !(resident[1] & 1)) result = 2;
    if (!result && mprotect(memory, page, PROT_NONE)) result = 3;
    if (!result && (mincore(memory, page, resident) || !(resident[0] & 1))) result = 4;
    if (!result && mprotect(memory, page, PROT_READ | PROT_WRITE)) result = 5;
    if (!result && (madvise(memory, 1, MADV_DONTNEED) || memory[0] != 0 || memory[page] != 89)) result = 6;
    if (munmap(memory, page * 2) && !result) result = 7;
    if (result) return result;
    struct sched_param parameter = { 0 };
    int policy = -1;
    errno = 57;
    if (pthread_getschedparam(pthread_self(), &policy, &parameter) || policy != SCHED_OTHER || parameter.sched_priority) return 8;
    if (pthread_setschedparam(pthread_self(), SCHED_OTHER, &parameter) || errno != 57) return 9;
    parameter.sched_priority = 1;
    if (pthread_setschedparam(pthread_self(), SCHED_FIFO, &parameter) != ENOTSUP) return 10;
    if (flock(-1, LOCK_EX | LOCK_NB) != -1 || errno != EBADF) return 11;
    Dl_info information;
    if (!dladdr((const void *)(uintptr_t)&sdk_unix_extensions_probe, &information) || !information.dli_fname || !information.dli_fbase) return 12;
    int called = 0;
    if (dl_iterate_phdr(stop, &called) != 17 || called != 1) return 13;
    return 0;
}
