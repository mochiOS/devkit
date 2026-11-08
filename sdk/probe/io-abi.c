/* C ABI contract: declarations must match the native runtime, not legacy newlib defaults. */
#if !defined(__mochios__) || defined(__linux__)
#error Native mochiOS SDK required
#endif
#define _LIBC 1
#include <sys/types.h>
#include <sys/stat.h>
#include <sys/uio.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <stddef.h>

typedef ssize_t (*read_signature)(int, void *, size_t);
typedef ssize_t (*write_signature)(int, const void *, size_t);
typedef ssize_t (*vector_signature)(int, const struct iovec *, int);
typedef int (*open_signature)(const char *, int, ...);
typedef int (*openat_signature)(int, const char *, int, ...);
typedef int (*chown_signature)(const char *, uid_t, gid_t);
typedef int (*fchown_signature)(int, uid_t, gid_t);
typedef int (*fchownat_signature)(int, const char *, uid_t, gid_t, int);
typedef int (*mkdir_signature)(const char *, mode_t);
typedef int (*mkdirat_signature)(int, const char *, mode_t);
#define ABI_MATCH(name, signature) \
    _Static_assert(_Generic(&(name), signature: 1, default: 0), #name " has the wrong public C signature")
ABI_MATCH(read, read_signature);
ABI_MATCH(write, write_signature);
ABI_MATCH(_read, read_signature);
ABI_MATCH(_write, write_signature);
ABI_MATCH(readv, vector_signature);
ABI_MATCH(writev, vector_signature);
ABI_MATCH(open, open_signature);
ABI_MATCH(_open, open_signature);
ABI_MATCH(openat, openat_signature);
ABI_MATCH(chown, chown_signature);
ABI_MATCH(lchown, chown_signature);
ABI_MATCH(fchown, fchown_signature);
ABI_MATCH(fchownat, fchownat_signature);
ABI_MATCH(mkdir, mkdir_signature);
ABI_MATCH(_mkdir, mkdir_signature);
ABI_MATCH(mkdirat, mkdirat_signature);
_Static_assert(sizeof(ssize_t) == 8 && sizeof(size_t) == 8, "64-bit I/O ABI required");
_Static_assert(sizeof(uid_t) == 2 && sizeof(gid_t) == 2, "Native credential ABI changed");
_Static_assert(sizeof(mode_t) == 4 && (mode_t)-1 > 0, "Native mode_t ABI changed");
_Static_assert(O_CREAT == 0x0200, "Native open vararg discriminator changed");

/* This is also callable by the native VM harness; cross-build success alone is not a runtime result. */
int sdk_posix_io_abi_probe(void) {
    char byte = 0;
    errno = 0;
    if (read(-1, &byte, 1) != (ssize_t)-1 || errno != EBADF) return 1;
    errno = 0;
    if (write(-1, &byte, 1) != (ssize_t)-1 || errno != EBADF) return 2;
    errno = 0;
    if (open(NULL, O_RDONLY) != -1 || errno != EFAULT) return 3;
    errno = 0;
    if (open(NULL, O_WRONLY | O_CREAT, (mode_t)0600) != -1 || errno != EFAULT) return 4;
    errno = 0;
    if (openat(AT_FDCWD, NULL, O_RDONLY) != -1 || errno != EFAULT) return 5;
    errno = 0;
    if (openat(AT_FDCWD, NULL, O_WRONLY | O_CREAT, (mode_t)0600) != -1 || errno != EFAULT) return 6;
    return 0;
}
