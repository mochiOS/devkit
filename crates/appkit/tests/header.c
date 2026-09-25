#include <mochios.h>

_Static_assert(sizeof(MochiosStringView) == 16, "MochiosStringView ABI changed");
_Static_assert(sizeof(MochiosMutableBuffer) == 16, "MochiosMutableBuffer ABI changed");
_Static_assert(MOCHIOS_ABI_VERSION == 0x00010000u, "unexpected AppKit ABI version");

static int compile_api_surface(void) {
    uint64_t process_id = 0;
    return mochios_document_open(
        mochios_c_string("/home/testuser/document.txt"),
        mochios_c_string("text/plain"),
        MOCHIOS_ASSOCIATION_ROLE_EDIT,
        &process_id
    );
}

int main(void) {
    return compile_api_surface();
}
