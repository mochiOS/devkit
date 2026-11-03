/* newlib's generic target does not build the BSD-specific global collation
 * tables used by posix/fnmatch.c. Use its public locale-aware strcoll instead.
 * This header is force-included only while compiling that pinned source. */
#ifndef MOCHIOS_NEWLIB_FNMATCH_COLLATE_H
#define MOCHIOS_NEWLIB_FNMATCH_COLLATE_H
#include <string.h>
#define _COLLATE_H_
#define __collate_load_error 0
static int __collate_range_cmp(int a, int b) {
    const char left[2] = {(char)a, 0};
    const char right[2] = {(char)b, 0};
    return strcoll(left, right);
}
#endif
