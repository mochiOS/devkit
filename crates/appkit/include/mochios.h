#ifndef MOCHIOS_H
#define MOCHIOS_H

#include <stddef.h>
#include <string.h>

#include "mochios_abi.h"
#include <viewkit.h>

static inline MochiosStringView mochios_string(const void *data, uint64_t length) {
    MochiosStringView value;
    value.data = (const uint8_t *)data;
    value.length = length;
    return value;
}

static inline MochiosStringView mochios_c_string(const char *value) {
    return value == NULL ? mochios_string(NULL, 0) : mochios_string(value, (uint64_t)strlen(value));
}

static inline MochiosMutableBuffer mochios_buffer(void *data, uint64_t capacity) {
    MochiosMutableBuffer value;
    value.data = (uint8_t *)data;
    value.capacity = capacity;
    return value;
}

#endif
