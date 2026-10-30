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
typedef struct pydoccraft_session pydoccraft_session;
pydoccraft_result pydoccraft_session_open(const uint8_t *, size_t, const uint8_t *, size_t,
    const uint8_t *, size_t, uint8_t, pydoccraft_session **);
pydoccraft_result pydoccraft_session_render(pydoccraft_session *, size_t, double, uint8_t,
    double, double, double, double);
pydoccraft_result pydoccraft_session_search(pydoccraft_session *, size_t, const uint8_t *, size_t);
/* Consumes the handle, even if close reports an error. Caller serializes all operations. */
pydoccraft_result pydoccraft_session_close(pydoccraft_session *);
typedef struct pydoccraft_epub pydoccraft_epub;
/* EPUB handles must be serialized by the caller and closed exactly once;
 * close consumes the handle even on error. Inputs are borrowed for the call. */
pydoccraft_result pydoccraft_epub_new(pydoccraft_epub **);
pydoccraft_result pydoccraft_epub_copy(pydoccraft_epub *, pydoccraft_epub **);
pydoccraft_result pydoccraft_epub_close(pydoccraft_epub *);
pydoccraft_result pydoccraft_epub_command(pydoccraft_epub *, const uint8_t *, size_t, const uint8_t *, size_t);
#endif
