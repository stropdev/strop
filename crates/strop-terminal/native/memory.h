/* Per-instance allocation ownership for the pinned VT engine. Only its one
 * worker thread uses this allocator. Refusal is sticky and reported by the
 * Rust owner; an internally handled OOM must never look like a complete frame. */
#include <ghostty/vt.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdbool.h>

#define STROP_VT_MEMORY_LIMIT (32u * 1024u * 1024u)
#define STROP_VT_ALLOCATION_LIMIT 65536u

typedef struct {
    size_t bytes;
    size_t allocations;
    bool failed;
} StropVtMemory;

static void *strop_vt_alloc(void *context, size_t length, uint8_t alignment, uintptr_t address) {
    (void)address;
    StropVtMemory *memory = context;
    size_t size = length ? length : 1;
    if (alignment > _Alignof(max_align_t) || size > STROP_VT_MEMORY_LIMIT - memory->bytes ||
        memory->allocations == STROP_VT_ALLOCATION_LIMIT) {
        memory->failed = true;
        return NULL;
    }
    void *result = malloc(size);
    if (!result) {
        memory->failed = true;
        return NULL;
    }
    memory->bytes += size;
    memory->allocations++;
    return result;
}

static bool strop_vt_resize_memory(void *context, void *pointer, size_t old_length,
                                    uint8_t alignment, size_t new_length, uintptr_t address) {
    (void)context; (void)pointer; (void)alignment; (void)address;
    /* No guessed malloc usable-size or falsely discounted retained storage. */
    return old_length == new_length;
}

static void *strop_vt_remap(void *context, void *pointer, size_t old_length,
                            uint8_t alignment, size_t new_length, uintptr_t address) {
    (void)address;
    StropVtMemory *memory = context;
    size_t old_size = old_length ? old_length : 1;
    size_t new_size = new_length ? new_length : 1;
    if (alignment > _Alignof(max_align_t) || old_size > memory->bytes ||
        new_size > STROP_VT_MEMORY_LIMIT - (memory->bytes - old_size)) {
        memory->failed = true;
        return NULL;
    }
    void *result = realloc(pointer, new_size);
    if (!result) {
        memory->failed = true;
        return NULL;
    }
    memory->bytes = memory->bytes - old_size + new_size;
    return result;
}

static void strop_vt_release_memory(void *context, void *pointer, size_t length,
                                    uint8_t alignment, uintptr_t address) {
    (void)alignment; (void)address;
    StropVtMemory *memory = context;
    size_t size = length ? length : 1;
    if (size > memory->bytes || memory->allocations == 0) {
        /* The allocator ABI supplies exact lengths. Keep corruption visible. */
        memory->failed = true;
    } else {
        memory->bytes -= size;
        memory->allocations--;
    }
    free(pointer);
}

static const GhosttyAllocatorVtable strop_vt_allocator_vtable = {
    .alloc = strop_vt_alloc,
    .resize = strop_vt_resize_memory,
    .remap = strop_vt_remap,
    .free = strop_vt_release_memory,
};
