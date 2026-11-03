/* Compile/link contract only; this program is never run on the build host. */
#if !defined(__mochios__) || defined(__linux__)
#error A real mochiOS target is required
#endif
#include <resolv.h>
#include <limits.h>
#include <utime.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <arpa/inet.h>
#include <fnmatch.h>
_Static_assert(sizeof(struct ip_mreq) == 8, "IPv4 multicast request ABI");
_Static_assert(sizeof(struct ipv6_mreq) == 20, "IPv6 multicast request ABI");
_Static_assert(IN_MULTICAST(0xe0000000U) && IN_MULTICAST(0xefffffffU), "multicast range");
_Static_assert(!IN_MULTICAST(0xdfffffffU) && !IN_MULTICAST(0xf0000000U), "multicast bounds");
_Static_assert(sizeof(struct sockaddr_storage) == 128, "socket storage size");
_Static_assert(_Alignof(struct sockaddr_storage) == 8, "socket storage alignment");
_Static_assert(sizeof(HEADER) == NS_HFIXEDSZ, "DNS header ABI");
_Static_assert(sizeof(struct utimbuf) == 16, "time_t ABI");
_Static_assert(SSIZE_MAX == (ssize_t)(SIZE_MAX >> 1), "ssize_t bound");
_Static_assert(HOST_NAME_MAX == 255, "native hostname bound");
int mochi_sdk_resolver_probe(void) {
    if (fnmatch("*.c", "probe.c", 0) != 0) return 1;
    struct in_addr legacy;
    if (!inet_aton("127.1", &legacy)) return 1;
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
