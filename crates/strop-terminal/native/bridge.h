#include <stddef.h>
#include <stdint.h>

typedef void (*StropVtEvent)(void *, uint32_t, const uint8_t *, size_t);
typedef struct {
    uint32_t codepoint, graphemes, width;
    uint32_t fg_kind, fg, bg_kind, bg, flags, wrapped;
} StropVtCell;
typedef struct {
    uint64_t cols, rows, cursor_x, cursor_y, visible, blinking, cursor_style;
    uint64_t alternate, history, modes, memory_bytes;
    int64_t previous_history_anchor;
} StropVtState;
typedef struct { uint8_t r, g, b; } StropVtRgb;
typedef struct {
    StropVtRgb foreground, background;
    StropVtRgb colors[256];
} StropVtPalette;

int strop_vt_new(uint16_t, uint16_t, size_t, size_t, void *, StropVtEvent, void **);
void strop_vt_free(void *);
int strop_vt_feed(void *, const uint8_t *, size_t);
int strop_vt_resize(void *, uint16_t, uint16_t);
int strop_vt_state(void *, StropVtState *);
int strop_vt_dirty_rows(void *, uint8_t *, size_t);
int strop_vt_palette(void *, StropVtPalette *);
int strop_vt_cell(void *, int32_t, uint16_t, StropVtCell *);
int strop_vt_graphemes(void *, int32_t, uint16_t, uint32_t *, size_t, size_t *);
int strop_vt_mark_history(void *);
int strop_vt_key(void *, const char *, uint32_t, uint32_t, uint32_t, uint32_t,
                 const char *, size_t, uint8_t *, size_t, size_t *);
int strop_vt_paste(void *, const uint8_t *, size_t, int);
int strop_vt_text(void *, uint32_t, uint8_t *, size_t, size_t *);
int strop_vt_focus(void *, int, uint8_t *, size_t, size_t *);
int strop_vt_keyboard(void *, uint8_t);
