/* Pinned libghostty-vt adapter. Native references never leave a synchronous
 * worker call; UI consumers receive owned Rust rows. No host clipboard, window,
 * process or filesystem action is performed by a VT callback. */
#include "bridge.h"
#include "memory.h"
#include <ghostty/vt.h>
#include <limits.h>
#include <string.h>

#ifndef STROP_TERMINAL_VERSION
#define STROP_TERMINAL_VERSION "strop"
#endif

#define CHECK(call) do { GhosttyResult result_ = (call); if (result_ != GHOSTTY_SUCCESS) return (int)result_; } while (0)

typedef struct {
    StropVtMemory memory;
    GhosttyAllocator allocator;
    GhosttyTerminal terminal;
    GhosttyRenderState render;
    GhosttyRenderStateRowIterator iterator;
    GhosttyTrackedGridRef history_anchor;
    GhosttyKeyEncoder encoder;
    GhosttyKeyEvent key;
    void *userdata;
    StropVtEvent event;
    GhosttyKittyKeyFlags keyboard_allowed;
    bool feeding;
} StropVt;

static void write_pty(GhosttyTerminal terminal, void *userdata, const uint8_t *data, size_t length) {
    (void)terminal;
    StropVt *state = userdata;
    /* The portable xterm-256color profile is the contract. Do not advertise
     * Ghostty-specific XTGETTCAP extensions the frontend cannot render. */
    if (state->feeding && length >= 5 && data[0] == 0x1b && data[1] == 'P' &&
        (data[2] == '0' || data[2] == '1') && data[3] == '+' && data[4] == 'r') return;
    /* The pinned handler emits a keyboard query reply as one complete callback.
     * Only filter VT-generated replies, never user paste chunks. */
    if (state->feeding && length >= 5 && length <= 6 &&
        data[0] == 0x1b && data[1] == '[' && data[2] == '?' &&
        data[length - 1] == 'u') {
        unsigned flags = 0;
        bool valid = true;
        for (size_t i = 3; i + 1 < length; i++) {
            if (data[i] < '0' || data[i] > '9') { valid = false; break; }
            flags = flags * 10 + data[i] - '0';
        }
        if (valid) {
            if (!state->keyboard_allowed) return;
            flags &= state->keyboard_allowed;
            uint8_t reply[6] = { 0x1b, '[', '?' };
            size_t count = 3;
            if (flags >= 10) reply[count++] = '0' + flags / 10;
            reply[count++] = '0' + flags % 10;
            reply[count++] = 'u';
            state->event(state->userdata, 0, reply, count);
            return;
        }
    }
    state->event(state->userdata, 0, data, length);
}
static void title_changed(GhosttyTerminal terminal, void *userdata) {
    (void)terminal;
    StropVt *state = userdata;
    state->event(state->userdata, 1, NULL, 0);
}
static void pwd_changed(GhosttyTerminal terminal, void *userdata) {
    (void)terminal;
    StropVt *state = userdata;
    state->event(state->userdata, 2, NULL, 0);
}
static void bell(GhosttyTerminal terminal, void *userdata) {
    (void)terminal;
    StropVt *state = userdata;
    state->event(state->userdata, 3, NULL, 0);
}
static void clipboard_write(GhosttyTerminal terminal, void *userdata, const GhosttyClipboardWrite *request) {
    (void)terminal;
    StropVt *state = userdata;
    state->event(state->userdata, 4, NULL, 0);
    GhosttyClipboardWriteReply reply = {
        .size = sizeof(reply),
        .result = GHOSTTY_CLIPBOARD_WRITE_RESULT_DENIED,
    };
    request->reply(request, &reply);
}
static GhosttyString version(GhosttyTerminal terminal, void *userdata) {
    (void)terminal; (void)userdata;
    return (GhosttyString){ .ptr = (const uint8_t *)STROP_TERMINAL_VERSION,
                            .len = sizeof(STROP_TERMINAL_VERSION) - 1 };
}

static bool device_attributes(GhosttyTerminal terminal, void *userdata, GhosttyDeviceAttributes *out) {
    (void)terminal; (void)userdata;
    *out = (GhosttyDeviceAttributes){
        .primary = { .conformance_level = GHOSTTY_DA_CONFORMANCE_VT220,
            .features = { GHOSTTY_DA_FEATURE_ANSI_COLOR }, .num_features = 1 },
        .secondary = { .device_type = GHOSTTY_DA_DEVICE_TYPE_VT220,
            .firmware_version = 0, .rom_cartridge = 0 },
        .tertiary = { .unit_id = 0 },
    };
    return true;
}

void strop_vt_free(void *handle) {
    StropVt *state = handle;
    if (!state) return;
    if (state->history_anchor) ghostty_tracked_grid_ref_free(state->history_anchor);
    if (state->iterator) ghostty_render_state_row_iterator_free(state->iterator);
    if (state->render) ghostty_render_state_free(state->render);
    if (state->key) ghostty_key_event_free(state->key);
    if (state->encoder) ghostty_key_encoder_free(state->encoder);
    if (state->terminal) ghostty_terminal_free(state->terminal);
    free(state);
}

int strop_vt_new(uint16_t columns, uint16_t rows, size_t history_lines,
                 size_t history_bytes, void *userdata, StropVtEvent event, void **out) {
    StropVt *state = calloc(1, sizeof(*state));
    if (!state) return -900;
    state->userdata = userdata;
    state->event = event;
    state->keyboard_allowed = GHOSTTY_KITTY_KEY_ALL;
    state->allocator = (GhosttyAllocator){ .ctx = &state->memory, .vtable = &strop_vt_allocator_vtable };
    GhosttyResult result = ghostty_terminal_new(&state->allocator, &state->terminal, columns, rows);
    if (result != GHOSTTY_SUCCESS) goto failed;
    result = ghostty_key_encoder_new(&state->allocator, &state->encoder);
    if (result != GHOSTTY_SUCCESS) goto failed;
    result = ghostty_key_event_new(&state->allocator, &state->key);
    if (result != GHOSTTY_SUCCESS) goto failed;
    result = ghostty_render_state_new(&state->allocator, &state->render);
    if (result != GHOSTTY_SUCCESS) goto failed;
    result = ghostty_render_state_row_iterator_new(&state->allocator, &state->iterator);
    if (result != GHOSTTY_SUCCESS) goto failed;
    size_t clipboard_bytes = 65536;
    GhosttyColorRgb foreground = { .r = 192, .g = 192, .b = 192 };
    GhosttyColorRgb background = { .r = 0, .g = 0, .b = 0 };
#define SET(option, value) do { result = ghostty_terminal_set(state->terminal, option, value); if (result != GHOSTTY_SUCCESS) goto failed; } while (0)
    SET(GHOSTTY_TERMINAL_OPT_USERDATA, state);
    SET(GHOSTTY_TERMINAL_OPT_WRITE_PTY, write_pty);
    SET(GHOSTTY_TERMINAL_OPT_TITLE_CHANGED, title_changed);
    SET(GHOSTTY_TERMINAL_OPT_PWD_CHANGED, pwd_changed);
    SET(GHOSTTY_TERMINAL_OPT_BELL, bell);
    SET(GHOSTTY_TERMINAL_OPT_XTVERSION, version);
    SET(GHOSTTY_TERMINAL_OPT_DEVICE_ATTRIBUTES, device_attributes);
    SET(GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE, clipboard_write);
    SET(GHOSTTY_TERMINAL_OPT_CLIPBOARD_WRITE_MAX_BYTES, &clipboard_bytes);
    /* No clipboard reader: reads are denied, and Kitty clipboard paste events
     * are not enabled by a callback that cannot serve them. */
    SET(GHOSTTY_TERMINAL_OPT_CLIPBOARD_READ, NULL);
    SET(GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES, &history_bytes);
    SET(GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES, &history_lines);
    SET(GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, &foreground);
    SET(GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, &background);
#undef SET
    *out = state;
    return 0;
failed:
    strop_vt_free(state);
    return (int)result;
}

int strop_vt_feed(void *handle, const uint8_t *data, size_t length) {
    StropVt *state = handle;
    state->feeding = true;
    ghostty_terminal_vt_write(state->terminal, data, length);
    state->feeding = false;
    return state->memory.failed ? -901 : 0;
}
int strop_vt_keyboard(void *handle, uint8_t allowed) {
    if (allowed & ~GHOSTTY_KITTY_KEY_ALL) return GHOSTTY_INVALID_VALUE;
    ((StropVt *)handle)->keyboard_allowed = allowed;
    return GHOSTTY_SUCCESS;
}
int strop_vt_resize(void *handle, uint16_t columns, uint16_t rows) {
    StropVt *state = handle;
    CHECK(ghostty_terminal_resize(state->terminal, columns, rows, 0, 0));
    return state->memory.failed ? -901 : 0;
}
static int mode(StropVt *state, uint16_t number, bool *out) {
    GhosttyTerminalModeConfig query = { .mode = ghostty_mode_new(number, false) };
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_MODE, &query));
    *out = query.value;
    return 0;
}

int strop_vt_state(void *handle, StropVtState *out) {
    StropVt *state = handle;
    CHECK(ghostty_render_state_update(state->render, state->terminal));
    uint16_t columns, rows, x, y;
    size_t history;
    uint8_t keyboard;
    GhosttyTerminalScreen screen;
    GhosttyRenderStateCursor cursor = GHOSTTY_INIT_SIZED(GhosttyRenderStateCursor);
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_COLS, &columns));
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_ROWS, &rows));
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_CURSOR_X, &x));
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_CURSOR_Y, &y));
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN, &screen));
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS, &history));
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS, &keyboard));
    CHECK(ghostty_render_state_get(state->render, GHOSTTY_RENDER_STATE_DATA_CURSOR, &cursor));
    bool application_cursor, bracketed_paste;
    if (mode(state, 1, &application_cursor) || mode(state, 2004, &bracketed_paste)) return -902;
    int64_t anchor = -1;
    if (state->history_anchor) {
        GhosttyPointCoordinate point;
        GhosttyResult result = ghostty_tracked_grid_ref_point(state->history_anchor, GHOSTTY_POINT_TAG_HISTORY, &point);
        if (result == GHOSTTY_SUCCESS) anchor = point.y;
        else if (result != GHOSTTY_NO_VALUE) return result;
    }
    *out = (StropVtState){ columns, rows, x, y, cursor.visible, cursor.blinking,
        cursor.visual_style, screen != GHOSTTY_TERMINAL_SCREEN_PRIMARY, history,
        (application_cursor ? 1 : 0) | (bracketed_paste ? 2 : 0) | (keyboard ? 4 : 0),
        state->memory.bytes, anchor };
    return state->memory.failed ? -901 : 0;
}

int strop_vt_dirty_rows(void *handle, uint8_t *out, size_t capacity) {
    StropVt *state = handle;
    memset(out, 0, capacity);
    CHECK(ghostty_render_state_get(state->render, GHOSTTY_RENDER_STATE_DATA_ROW_ITERATOR, &state->iterator));
    uint16_t row;
    while (ghostty_render_state_row_iterator_next_dirty(state->iterator, &row)) {
        if (row >= capacity) return -903;
        out[row] = 1;
    }
    return 0;
}

int strop_vt_palette(void *handle, StropVtPalette *out) {
    StropVt *state = handle;
    GhosttyRenderStateColors colors = GHOSTTY_INIT_SIZED(GhosttyRenderStateColors);
    CHECK(ghostty_render_state_get(state->render, GHOSTTY_RENDER_STATE_DATA_COLORS, &colors));
    out->foreground = (StropVtRgb){ colors.foreground.r, colors.foreground.g, colors.foreground.b };
    out->background = (StropVtRgb){ colors.background.r, colors.background.g, colors.background.b };
    for (size_t index = 0; index < 256; index++) {
        out->colors[index] = (StropVtRgb){ colors.palette[index].r, colors.palette[index].g, colors.palette[index].b };
    }
    return 0;
}
int strop_vt_palette_set(void *handle, const StropVtPalette *palette) {
    StropVt *state = handle;
    GhosttyColorRgb foreground = { palette->foreground.r, palette->foreground.g, palette->foreground.b };
    GhosttyColorRgb background = { palette->background.r, palette->background.g, palette->background.b };
    GhosttyColorRgb colors[256];
    for (size_t index = 0; index < 256; index++) {
        colors[index] = (GhosttyColorRgb){ palette->colors[index].r, palette->colors[index].g, palette->colors[index].b };
    }
    /* The palette set preserves per-index OSC overrides a program already
     * applied; only unmodified indices adopt the new defaults. */
    CHECK(ghostty_terminal_set(state->terminal, GHOSTTY_TERMINAL_OPT_COLOR_FOREGROUND, &foreground));
    CHECK(ghostty_terminal_set(state->terminal, GHOSTTY_TERMINAL_OPT_COLOR_BACKGROUND, &background));
    CHECK(ghostty_terminal_set(state->terminal, GHOSTTY_TERMINAL_OPT_COLOR_PALETTE, &colors));
    return state->memory.failed ? -901 : 0;
}

static int grid_ref(StropVt *state, int32_t row, uint16_t column, GhosttyGridRef *out) {
    GhosttyPoint point = { .tag = GHOSTTY_POINT_TAG_ACTIVE,
        .value = { .coordinate = { .x = column, .y = (uint32_t)row } } };
    if (row < 0) {
        size_t history;
        CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS, &history));
        if ((uint64_t)(-(int64_t)row) > history || history > UINT32_MAX) return -904;
        point.tag = GHOSTTY_POINT_TAG_HISTORY;
        point.value.coordinate.y = (uint32_t)(history - (uint64_t)(-(int64_t)row));
    }
    return ghostty_terminal_grid_ref(state->terminal, point, out);
}
static uint32_t color_value(GhosttyStyleColor color) {
    if (color.tag == GHOSTTY_STYLE_COLOR_PALETTE) return color.value.palette;
    if (color.tag == GHOSTTY_STYLE_COLOR_RGB) {
        return ((uint32_t)color.value.rgb.r << 16) | ((uint32_t)color.value.rgb.g << 8) | color.value.rgb.b;
    }
    return 0;
}

int strop_vt_cell(void *handle, int32_t row, uint16_t column, StropVtCell *out) {
    StropVt *state = handle;
    GhosttyGridRef ref = GHOSTTY_INIT_SIZED(GhosttyGridRef);
    CHECK(grid_ref(state, row, column, &ref));
    GhosttyCell cell;
    GhosttyCellWide wide;
    GhosttyCellContentTag content;
    GhosttyStyle style = GHOSTTY_INIT_SIZED(GhosttyStyle);
    GhosttyRow native_row;
    bool wrapped;
    uint32_t codepoint;
    CHECK(ghostty_grid_ref_cell(&ref, &cell));
    CHECK(ghostty_cell_get(cell, GHOSTTY_CELL_DATA_WIDE, &wide));
    CHECK(ghostty_cell_get(cell, GHOSTTY_CELL_DATA_CODEPOINT, &codepoint));
    CHECK(ghostty_cell_get(cell, GHOSTTY_CELL_DATA_CONTENT_TAG, &content));
    CHECK(ghostty_grid_ref_style(&ref, &style));
    CHECK(ghostty_grid_ref_row(&ref, &native_row));
    CHECK(ghostty_row_get(native_row, GHOSTTY_ROW_DATA_WRAP, &wrapped));
    size_t count = 0;
    GhosttyResult result = ghostty_grid_ref_graphemes(&ref, NULL, 0, &count);
    if (result != GHOSTTY_SUCCESS && result != GHOSTTY_OUT_OF_SPACE) return result;
    if (count > UINT32_MAX) return -905;
    uint32_t bg_kind = style.bg_color.tag;
    uint32_t bg = color_value(style.bg_color);
    if (content == GHOSTTY_CELL_CONTENT_BG_COLOR_PALETTE) {
        GhosttyColorPaletteIndex index;
        CHECK(ghostty_cell_get(cell, GHOSTTY_CELL_DATA_COLOR_PALETTE, &index));
        bg_kind = GHOSTTY_STYLE_COLOR_PALETTE;
        bg = index;
    } else if (content == GHOSTTY_CELL_CONTENT_BG_COLOR_RGB) {
        GhosttyColorRgb rgb;
        CHECK(ghostty_cell_get(cell, GHOSTTY_CELL_DATA_COLOR_RGB, &rgb));
        bg_kind = GHOSTTY_STYLE_COLOR_RGB;
        bg = ((uint32_t)rgb.r << 16) | ((uint32_t)rgb.g << 8) | rgb.b;
    }
    *out = (StropVtCell){ codepoint, (uint32_t)count,
        wide == GHOSTTY_CELL_WIDE_WIDE ? 2 : wide == GHOSTTY_CELL_WIDE_SPACER_TAIL ? 0 : 1,
        style.fg_color.tag, color_value(style.fg_color), bg_kind, bg,
        (style.bold ? 1 : 0) | (style.italic ? 2 : 0) | (style.underline ? 4 : 0) |
        (style.inverse ? 8 : 0) | (style.faint ? 16 : 0) | (style.invisible ? 32 : 0) |
        (style.strikethrough ? 64 : 0), wrapped };
    return 0;
}

int strop_vt_graphemes(void *handle, int32_t row, uint16_t column,
                       uint32_t *out, size_t capacity, size_t *written) {
    StropVt *state = handle;
    GhosttyGridRef ref = GHOSTTY_INIT_SIZED(GhosttyGridRef);
    CHECK(grid_ref(state, row, column, &ref));
    return ghostty_grid_ref_graphemes(&ref, out, capacity, written);
}

int strop_vt_mark_history(void *handle) {
    StropVt *state = handle;
    size_t history;
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS, &history));
    if (history == 0) {
        if (state->history_anchor) ghostty_tracked_grid_ref_free(state->history_anchor);
        state->history_anchor = NULL;
    } else {
        if (history > UINT32_MAX) return -904;
        GhosttyPoint point = { .tag = GHOSTTY_POINT_TAG_HISTORY,
            .value = { .coordinate = { .x = 0, .y = (uint32_t)(history - 1) } } };
        if (state->history_anchor) {
            CHECK(ghostty_tracked_grid_ref_set(state->history_anchor, state->terminal, point));
        } else {
            CHECK(ghostty_terminal_grid_ref_track(state->terminal, point, &state->history_anchor));
        }
    }
    CHECK(ghostty_render_state_clean(state->render));
    return state->memory.failed ? -901 : 0;
}

static GhosttyKey named_key(const char *name, uint32_t scalar, bool keypad) {
    if (keypad && scalar >= '0' && scalar <= '9') return GHOSTTY_KEY_NUMPAD_0 + scalar - '0';
    if (scalar >= 'a' && scalar <= 'z') return GHOSTTY_KEY_A + scalar - 'a';
    if (scalar >= 'A' && scalar <= 'Z') return GHOSTTY_KEY_A + scalar - 'A';
    if (scalar >= '0' && scalar <= '9') return GHOSTTY_KEY_DIGIT_0 + scalar - '0';
#define KEY(name_, value) if (!strcmp(name, name_)) return value
    KEY("Escape", GHOSTTY_KEY_ESCAPE); KEY("Enter", keypad ? GHOSTTY_KEY_NUMPAD_ENTER : GHOSTTY_KEY_ENTER);
    KEY("Backspace", GHOSTTY_KEY_BACKSPACE); KEY("Tab", GHOSTTY_KEY_TAB);
    KEY("Up", keypad ? GHOSTTY_KEY_NUMPAD_UP : GHOSTTY_KEY_ARROW_UP);
    KEY("Down", keypad ? GHOSTTY_KEY_NUMPAD_DOWN : GHOSTTY_KEY_ARROW_DOWN);
    KEY("Left", keypad ? GHOSTTY_KEY_NUMPAD_LEFT : GHOSTTY_KEY_ARROW_LEFT);
    KEY("Right", keypad ? GHOSTTY_KEY_NUMPAD_RIGHT : GHOSTTY_KEY_ARROW_RIGHT);
    KEY("Home", keypad ? GHOSTTY_KEY_NUMPAD_HOME : GHOSTTY_KEY_HOME);
    KEY("End", keypad ? GHOSTTY_KEY_NUMPAD_END : GHOSTTY_KEY_END);
    KEY("PageUp", keypad ? GHOSTTY_KEY_NUMPAD_PAGE_UP : GHOSTTY_KEY_PAGE_UP);
    KEY("PageDown", keypad ? GHOSTTY_KEY_NUMPAD_PAGE_DOWN : GHOSTTY_KEY_PAGE_DOWN);
    KEY("Insert", keypad ? GHOSTTY_KEY_NUMPAD_INSERT : GHOSTTY_KEY_INSERT);
    KEY("Delete", keypad ? GHOSTTY_KEY_NUMPAD_DELETE : GHOSTTY_KEY_DELETE);
    KEY("CapsLock", GHOSTTY_KEY_CAPS_LOCK); KEY("NumLock", GHOSTTY_KEY_NUM_LOCK);
    KEY("ScrollLock", GHOSTTY_KEY_SCROLL_LOCK); KEY("PrintScreen", GHOSTTY_KEY_PRINT_SCREEN);
    KEY("Pause", GHOSTTY_KEY_PAUSE); KEY("Menu", GHOSTTY_KEY_CONTEXT_MENU);
    KEY("LeftShift", GHOSTTY_KEY_SHIFT_LEFT); KEY("RightShift", GHOSTTY_KEY_SHIFT_RIGHT);
    KEY("LeftControl", GHOSTTY_KEY_CONTROL_LEFT); KEY("RightControl", GHOSTTY_KEY_CONTROL_RIGHT);
    KEY("LeftAlt", GHOSTTY_KEY_ALT_LEFT); KEY("RightAlt", GHOSTTY_KEY_ALT_RIGHT);
    KEY("LeftSuper", GHOSTTY_KEY_META_LEFT); KEY("RightSuper", GHOSTTY_KEY_META_RIGHT);
    KEY("KeypadBegin", GHOSTTY_KEY_NUMPAD_BEGIN);
    KEY("MediaPlayPause", GHOSTTY_KEY_MEDIA_PLAY_PAUSE);
    KEY("MediaStop", GHOSTTY_KEY_MEDIA_STOP);
    KEY("MediaTrackNext", GHOSTTY_KEY_MEDIA_TRACK_NEXT);
    KEY("MediaTrackPrevious", GHOSTTY_KEY_MEDIA_TRACK_PREVIOUS);
    KEY("LowerVolume", GHOSTTY_KEY_AUDIO_VOLUME_DOWN);
    KEY("RaiseVolume", GHOSTTY_KEY_AUDIO_VOLUME_UP);
    KEY("MuteVolume", GHOSTTY_KEY_AUDIO_VOLUME_MUTE);
#undef KEY
    if (name[0] == 'F') {
        char *end;
        long number = strtol(name + 1, &end, 10);
        if (!*end && number >= 1 && number <= 25) return GHOSTTY_KEY_F1 + number - 1;
    }
    switch (scalar) {
    case ' ': return GHOSTTY_KEY_SPACE;
    case '`': return GHOSTTY_KEY_BACKQUOTE;
    case '\\': return GHOSTTY_KEY_BACKSLASH;
    case '[': return GHOSTTY_KEY_BRACKET_LEFT;
    case ']': return GHOSTTY_KEY_BRACKET_RIGHT;
    case ',': return keypad ? GHOSTTY_KEY_NUMPAD_COMMA : GHOSTTY_KEY_COMMA;
    case '=': return keypad ? GHOSTTY_KEY_NUMPAD_EQUAL : GHOSTTY_KEY_EQUAL;
    case '-': return keypad ? GHOSTTY_KEY_NUMPAD_SUBTRACT : GHOSTTY_KEY_MINUS;
    case '+': return keypad ? GHOSTTY_KEY_NUMPAD_ADD : GHOSTTY_KEY_UNIDENTIFIED;
    case '*': return keypad ? GHOSTTY_KEY_NUMPAD_MULTIPLY : GHOSTTY_KEY_UNIDENTIFIED;
    case '.': return keypad ? GHOSTTY_KEY_NUMPAD_DECIMAL : GHOSTTY_KEY_PERIOD;
    case '\'': return GHOSTTY_KEY_QUOTE;
    case ';': return GHOSTTY_KEY_SEMICOLON;
    case '/': return keypad ? GHOSTTY_KEY_NUMPAD_DIVIDE : GHOSTTY_KEY_SLASH;
    default: return GHOSTTY_KEY_UNIDENTIFIED;
    }
}

int strop_vt_key(void *handle, const char *name, uint32_t scalar, uint32_t modifiers,
                  uint32_t action, uint32_t state_flags, const char *text, size_t length,
                  uint8_t *out, size_t capacity, size_t *written) {
    StropVt *state = handle;
    GhosttyKey key = named_key(name, scalar, (state_flags & 1) != 0);
    if (!scalar && key == GHOSTTY_KEY_UNIDENTIFIED) return -906;
    GhosttyMods mods = ((modifiers & 1) ? GHOSTTY_MODS_SHIFT : 0) |
        ((modifiers & 2) ? GHOSTTY_MODS_ALT : 0) |
        ((modifiers & 4) ? GHOSTTY_MODS_CTRL : 0) |
        ((modifiers & 8) ? GHOSTTY_MODS_SUPER : 0) |
        ((state_flags & 2) ? GHOSTTY_MODS_CAPS_LOCK : 0) |
        ((state_flags & 4) ? GHOSTTY_MODS_NUM_LOCK : 0);
    ghostty_key_event_set_key(state->key, key);
    ghostty_key_event_set_mods(state->key, mods);
    ghostty_key_event_set_consumed_mods(state->key, 0);
    ghostty_key_event_set_action(state->key, action == 0 ? GHOSTTY_KEY_ACTION_PRESS :
        action == 1 ? GHOSTTY_KEY_ACTION_REPEAT : GHOSTTY_KEY_ACTION_RELEASE);
    ghostty_key_event_set_utf8(state->key, text, length);
    ghostty_key_event_set_unshifted_codepoint(state->key,
        (modifiers & 1) && scalar >= 'A' && scalar <= 'Z' ? scalar + 32 : scalar);
    ghostty_key_encoder_setopt_from_terminal(state->encoder, state->terminal);
    GhosttyKittyKeyFlags keyboard;
    CHECK(ghostty_terminal_get(state->terminal, GHOSTTY_TERMINAL_DATA_KITTY_KEYBOARD_FLAGS, &keyboard));
    keyboard &= state->keyboard_allowed;
    ghostty_key_encoder_setopt(state->encoder, GHOSTTY_KEY_ENCODER_OPT_KITTY_FLAGS, &keyboard);
    CHECK(ghostty_key_encoder_encode(state->encoder, state->key, (char *)out, capacity, written));
    return state->memory.failed ? -901 : 0;
}

typedef struct { const uint8_t *data; size_t length; } StropPaste;
static bool read_paste(void *userdata, GhosttyString mime, GhosttyWriter writer) {
    (void)mime;
    StropPaste *paste = userdata;
    return writer.write(writer.userdata, paste->data, paste->length);
}
int strop_vt_paste(void *handle, const uint8_t *data, size_t length, int confirmed) {
    StropVt *state = handle;
    StropPaste source = { data, length };
    GhosttyString mime = { .ptr = (const uint8_t *)"text/plain", .len = 10 };
    GhosttyPaste paste = { .size = sizeof(paste), .location = GHOSTTY_CLIPBOARD_LOCATION_STANDARD,
        .source = GHOSTTY_PASTE_SOURCE_CLIPBOARD, .mimes = &mime, .mimes_len = 1,
        .reader = { .read = read_paste, .userdata = &source }, .allow_unsafe = confirmed != 0 };
    bool written;
    CHECK(ghostty_terminal_paste(state->terminal, &paste, &written));
    return state->memory.failed ? -901 : 0;
}

int strop_vt_focus(void *handle, int focused, uint8_t *out, size_t capacity, size_t *written) {
    StropVt *state = handle;
    bool reporting;
    CHECK(mode(state, 1004, &reporting));
    *written = 0;
    if (!reporting) return 0;
    return ghostty_focus_encode(focused ? GHOSTTY_FOCUS_GAINED : GHOSTTY_FOCUS_LOST,
        (char *)out, capacity, written);
}

int strop_vt_text(void *handle, uint32_t kind, uint8_t *out, size_t capacity, size_t *written) {
    StropVt *state = handle;
    GhosttyString text;
    CHECK(ghostty_terminal_get(state->terminal,
        kind == 1 ? GHOSTTY_TERMINAL_DATA_TITLE : GHOSTTY_TERMINAL_DATA_PWD, &text));
    if (text.len > capacity) return -907;
    if (text.len) memcpy(out, text.ptr, text.len);
    *written = text.len;
    return 0;
}
