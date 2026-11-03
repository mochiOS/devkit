/* Compile/link contract only; this program is never run on the build host. */
#if !defined(__mochios__) || defined(__linux__)
#error A real mochiOS target is required
#endif
#include <resolv.h>
#include <limits.h>
#include <utime.h>
_Static_assert(sizeof(HEADER) == NS_HFIXEDSZ, "DNS header ABI");
_Static_assert(sizeof(struct utimbuf) == 16, "time_t ABI");
_Static_assert(SSIZE_MAX == (ssize_t)(SIZE_MAX >> 1), "ssize_t bound");
_Static_assert(HOST_NAME_MAX == 255, "native hostname bound");
int mochi_sdk_resolver_probe(void) {
    unsigned char answer[4096];
    char name[NS_MAXDNAME];
    int n = res_query("example.invalid", C_IN, T_MX, answer, sizeof(answer));
    if (n < 0) return h_errno;
    if (n > (int)sizeof(answer) || n < NS_HFIXEDSZ) return 1;
    const unsigned char *cursor = answer + 4;
    unsigned short count;
    GETSHORT(count, cursor);
    if (!count) return utime("example.invalid", 0);
    return dn_expand(answer, answer + n, answer + NS_HFIXEDSZ, name, sizeof(name));
}
