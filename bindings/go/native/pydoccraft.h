#ifndef PYDOCCRAFT_H
#define PYDOCCRAFT_H
#include <stddef.h>
#include <stdint.h>
typedef struct { uint8_t *ptr; size_t len; } pydoccraft_buffer;
typedef struct { uint32_t status; pydoccraft_buffer data; } pydoccraft_result;
/* Nonempty input buffers must be readable throughout the call. Returned buffers
 * are owned by Rust and must be freed exactly once, unchanged, even on error. */
pydoccraft_result pydoccraft_detect(const uint8_t *, size_t);
pydoccraft_result pydoccraft_convert(const uint8_t *, size_t, const uint8_t *, size_t,
    const uint8_t *, size_t, const uint8_t *, size_t, uint8_t);
pydoccraft_result pydoccraft_extract(const uint8_t *, size_t, const uint8_t *, size_t,
    const uint8_t *, size_t, uint8_t);
pydoccraft_result pydoccraft_render(const uint8_t *, size_t, const uint8_t *, size_t,
    const uint8_t *, size_t, uint8_t, size_t, double, uint8_t, double, double, double, double);
void pydoccraft_free(pydoccraft_buffer);
#endif
