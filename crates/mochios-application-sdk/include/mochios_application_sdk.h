#ifndef MOCHIOS_APPLICATION_SDK_H
#define MOCHIOS_APPLICATION_SDK_H

#include <stddef.h>
#include <string.h>

#include "mochios_application_sdk_abi.h"
#include <viewkit.h>

static inline MoSdkStringView mosdk_string(const void *data, uint64_t length) {
    MoSdkStringView value;
    value.data = (const uint8_t *)data;
    value.length = length;
    return value;
}

static inline MoSdkStringView mosdk_c_string(const char *value) {
    return value == NULL ? mosdk_string(NULL, 0) : mosdk_string(value, (uint64_t)strlen(value));
}

static inline MoSdkMutableBuffer mosdk_buffer(void *data, uint64_t capacity) {
    MoSdkMutableBuffer value;
    value.data = (uint8_t *)data;
    value.capacity = capacity;
    return value;
}

#endif
