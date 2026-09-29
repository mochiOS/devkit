#ifndef MOCHIOS_ABI_H
#define MOCHIOS_ABI_H

#include <stdint.h>

#define MOCHIOS_ABI_VERSION_MAJOR 1u
#define MOCHIOS_ABI_VERSION_MINOR 0u
#define MOCHIOS_ABI_VERSION_PATCH 0u
#define MOCHIOS_ABI_VERSION ((MOCHIOS_ABI_VERSION_MAJOR << 16) | (MOCHIOS_ABI_VERSION_MINOR << 8) | MOCHIOS_ABI_VERSION_PATCH)

#define MOCHIOS_STATUS_OK 0
#define MOCHIOS_STATUS_NULL_POINTER 1
#define MOCHIOS_STATUS_INVALID_UTF8 2
#define MOCHIOS_STATUS_INVALID_ARGUMENT 3
#define MOCHIOS_STATUS_BUFFER_TOO_SMALL 4
#define MOCHIOS_STATUS_UNSUPPORTED_PLATFORM 5
#define MOCHIOS_STATUS_SYSTEM_ERROR 6
#define MOCHIOS_STATUS_PANIC 255

#define MOCHIOS_ASSOCIATION_ROLE_VIEW ((uint16_t)1u << 0)
#define MOCHIOS_ASSOCIATION_ROLE_EDIT ((uint16_t)1u << 1)
#define MOCHIOS_ASSOCIATION_ROLE_ALL (MOCHIOS_ASSOCIATION_ROLE_VIEW | MOCHIOS_ASSOCIATION_ROLE_EDIT)

typedef struct MochiosStringView {
    const uint8_t *data;
    uint64_t length;
} MochiosStringView;

typedef struct MochiosMutableBuffer {
    uint8_t *data;
    uint64_t capacity;
} MochiosMutableBuffer;

#ifdef __cplusplus
extern "C" {
#endif

uint32_t mochios_abi_version(void);
int64_t mochios_last_system_error(void);
MochiosStringView mochios_status_name(int32_t status);

int32_t mochios_clipboard_set_text(MochiosStringView text);
int32_t mochios_clipboard_copy_text(MochiosMutableBuffer output, uint64_t *required_length, uint8_t *has_text);

int32_t mochios_association_set(MochiosStringView extension, MochiosStringView content_type, MochiosStringView bundle_id, uint16_t role_bits);
int32_t mochios_association_remove(MochiosStringView extension, MochiosStringView content_type, uint16_t role_bits);
int32_t mochios_association_resolve(MochiosStringView extension, MochiosStringView content_type, uint16_t role_bits, MochiosMutableBuffer output, uint64_t *required_length);

int32_t mochios_document_open(MochiosStringView path, MochiosStringView content_type, uint16_t role_bits, uint64_t *process_id);
int32_t mochios_document_open_with(MochiosStringView path, MochiosStringView content_type, MochiosStringView bundle_id, uint16_t role_bits, uint64_t *process_id);
int32_t mochios_application_request_exit(void);

#ifdef __cplusplus
}
#endif

#endif
