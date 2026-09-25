#ifndef MOCHIOS_APPLICATION_SDK_ABI_H
#define MOCHIOS_APPLICATION_SDK_ABI_H

#include <stdint.h>

#define MOSDK_ABI_VERSION_MAJOR 1u
#define MOSDK_ABI_VERSION_MINOR 0u
#define MOSDK_ABI_VERSION_PATCH 0u
#define MOSDK_ABI_VERSION ((MOSDK_ABI_VERSION_MAJOR << 16) | (MOSDK_ABI_VERSION_MINOR << 8) | MOSDK_ABI_VERSION_PATCH)

#define MOSDK_STATUS_OK 0
#define MOSDK_STATUS_NULL_POINTER 1
#define MOSDK_STATUS_INVALID_UTF8 2
#define MOSDK_STATUS_INVALID_ARGUMENT 3
#define MOSDK_STATUS_BUFFER_TOO_SMALL 4
#define MOSDK_STATUS_UNSUPPORTED_PLATFORM 5
#define MOSDK_STATUS_SYSTEM_ERROR 6
#define MOSDK_STATUS_PANIC 255

#define MOSDK_ASSOCIATION_ROLE_VIEW ((uint16_t)1u << 0)
#define MOSDK_ASSOCIATION_ROLE_EDIT ((uint16_t)1u << 1)
#define MOSDK_ASSOCIATION_ROLE_ALL (MOSDK_ASSOCIATION_ROLE_VIEW | MOSDK_ASSOCIATION_ROLE_EDIT)

typedef struct MoSdkStringView {
    const uint8_t *data;
    uint64_t length;
} MoSdkStringView;

typedef struct MoSdkMutableBuffer {
    uint8_t *data;
    uint64_t capacity;
} MoSdkMutableBuffer;

#ifdef __cplusplus
extern "C" {
#endif

uint32_t mosdk_abi_version(void);
int64_t mosdk_last_system_error(void);
MoSdkStringView mosdk_status_name(int32_t status);

int32_t mosdk_clipboard_set_text(MoSdkStringView text);
int32_t mosdk_clipboard_copy_text(MoSdkMutableBuffer output, uint64_t *required_length, uint8_t *has_text);

int32_t mosdk_association_set(MoSdkStringView extension, MoSdkStringView content_type, MoSdkStringView bundle_id, uint16_t role_bits);
int32_t mosdk_association_remove(MoSdkStringView extension, MoSdkStringView content_type, uint16_t role_bits);
int32_t mosdk_association_resolve(MoSdkStringView extension, MoSdkStringView content_type, uint16_t role_bits, MoSdkMutableBuffer output, uint64_t *required_length);

int32_t mosdk_document_open(MoSdkStringView path, MoSdkStringView content_type, uint16_t role_bits, uint64_t *process_id);
int32_t mosdk_document_open_with(MoSdkStringView path, MoSdkStringView content_type, MoSdkStringView bundle_id, uint16_t role_bits, uint64_t *process_id);
int32_t mosdk_application_request_exit(void);

#ifdef __cplusplus
}
#endif

#endif
