/* LLVM libunwind registers one FDE per __register_frame call. All state belongs
 * to the executable's one runtime; a DSO never creates another FD/errno table. */
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
extern const unsigned char __mochios_dso_eh_start[], __mochios_dso_eh_end[];
extern void __register_frame(const void *);
extern void __deregister_frame(const void *);
extern void __cxa_finalize(void *) __attribute__((weak));
__attribute__((visibility("hidden"))) void *__dso_handle = &__dso_handle;
static void visit(int remove) {
    uintptr_t p = (uintptr_t)__mochios_dso_eh_start;
    const uintptr_t end = (uintptr_t)__mochios_dso_eh_end;
    while (p < end) {
        if (end - p < 4) abort();
        uint32_t small; __builtin_memcpy(&small, (const void *)p, 4);
        if (!small) break;
        uint64_t length = small; size_t header = 4, id_size = 4;
        if (small == UINT32_MAX) {
            if (end - p < 12) abort();
            __builtin_memcpy(&length, (const void *)(p + 4), 8);
            header = 12; id_size = 8;
        }
        if (length < id_size || length > end - p - header) abort();
        uint64_t id = 0; __builtin_memcpy(&id, (const void *)(p + header), id_size);
        if (id) {
            if (remove) __deregister_frame((const void *)p);
            else __register_frame((const void *)p);
        }
        p += header + (uintptr_t)length;
    }
}
__attribute__((constructor(101))) static void register_frames(void) { visit(0); }
__attribute__((destructor(101))) static void unregister_frames(void) {
    if (__cxa_finalize) __cxa_finalize(__dso_handle);
    visit(1);
}
