/* malloc.h —— BORUIX libc 堆分配器。 */
#ifndef _BORUIX_MALLOC_H
#define _BORUIX_MALLOC_H

#include "boruix_ctypes.h"

#ifdef __cplusplus
extern "C" {
#endif

void *malloc(size_t size);
void free(void *ptr);
void *realloc(void *ptr, size_t new_size);
void *calloc(size_t nmemb, size_t size);
size_t malloc_usable_size(void *ptr);
int posix_memalign(void **memptr, size_t alignment, size_t size);
void *aligned_alloc(size_t alignment, size_t size);

#ifdef __cplusplus
}
#endif

#endif /* _BORUIX_MALLOC_H */
