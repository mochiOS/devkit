#include <mochios_application_sdk.h>

_Static_assert(sizeof(MoSdkStringView) == 16, "MoSdkStringView ABI changed");
_Static_assert(sizeof(MoSdkMutableBuffer) == 16, "MoSdkMutableBuffer ABI changed");
_Static_assert(MOSDK_ABI_VERSION == 0x00010000u, "unexpected SDK ABI version");

static int compile_api_surface(void) {
    uint64_t process_id = 0;
    return mosdk_document_open(
        mosdk_c_string("/home/testuser/document.txt"),
        mosdk_c_string("text/plain"),
        MOSDK_ASSOCIATION_ROLE_EDIT,
        &process_id
    );
}

int main(void) {
    return compile_api_surface();
}
