#include <dlfcn.h>
#include <stdlib.h>
/* Requires this private DSO to be installed and declared by a native fixture.
 * Failure is a real failure; compile/link alone is not a runtime pass. */
int sdk_loader_probe(void) {
    /* Force a real dynamic-ABI reference to the fail-closed environment API. */
    char * volatile environment = secure_getenv("MOCHIOS_SDK_ENVIRONMENT_PROBE");
    (void)environment;
    void *library = dlopen("/system/libraries/libsdk_loader_probe_private.so", RTLD_NOW | RTLD_LOCAL);
    if (!library) { (void)dlerror(); return 1; }
    int (*value)(void) = (int (*)(void))dlsym(library, "sdk_loader_private_value");
    if (!value) { (void)dlerror(); dlclose(library); return 2; }
    int result = value() == 42 ? 0 : 3;
    return dlclose(library) == 0 ? result : 4;
}
