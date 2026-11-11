/* Compile/link only. Successful resource access additionally requires the
 * matching OS brokers, signed file bindings and launcher-installed authority. */
#if !defined(__mochios__) || defined(__linux__)
#error Native mochiOS SDK required
#endif
#include <mochi/bundle.h>
#include <stddef.h>
#include <stdint.h>
typedef int (*check_signature)(const char *);
typedef int (*open_signature)(const char *, const char *);
typedef uint64_t (*directory_signature)(const char *, char *, size_t);
typedef int (*close_signature)(uint64_t);
#define ABI_MATCH(name, signature) \
    _Static_assert(_Generic(&(name), signature: 1, default: 0), #name " ABI mismatch")
ABI_MATCH(mochi_bundle_check, check_signature);
ABI_MATCH(mochi_bundle_open, open_signature);
ABI_MATCH(mochi_bundle_directory_open, directory_signature);
ABI_MATCH(mochi_bundle_directory_close, close_signature);

/* All imports remain strong references, including functions that a runtime
 * access failure would otherwise skip. Nothing here simulates success. */
int sdk_bundle_probe(void) {
    char path[128];
    volatile int selected = mochi_bundle_check("");
    volatile int file = mochi_bundle_open("", "appicon.png");
    uint64_t view = mochi_bundle_directory_open("", path, sizeof path);
    return selected + file + mochi_bundle_directory_close(view);
}
