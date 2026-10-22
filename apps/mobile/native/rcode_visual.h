#ifndef RCODE_VISUAL_H
#define RCODE_VISUAL_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct RCodeVisualBytes {
  uint8_t *data;
  size_t len;
} RCodeVisualBytes;

/* Input strings are borrowed, UTF-8 and NUL terminated for the call. Font
 * bytes are copied once per face for process lifetime. Every exported function
 * contains its Rust panic boundary; zero/NULL indicates no result or failure. */
bool rcode_visual_font(uint8_t face, const uint8_t *data, size_t len);
uint64_t rcode_visual_create(const char *config_json);

/* Polls coalesced display/frames/measurement requests. Returned strings are
 * owned and must be released with string_free. Images are addressed by the
 * returned token until the next poll of the same instance; take each at most
 * once. Node ids are decimal strings to preserve all 64 bits in Dart. */
char *rcode_visual_poll(uint64_t visual);
RCodeVisualBytes rcode_visual_image(uint64_t visual, uint64_t token);
void rcode_visual_bytes_free(RCodeVisualBytes bytes);
void rcode_visual_command(uint64_t visual, const char *command_json);

/* Idempotently closes source networking, wakes pending measurements, and
 * signals worker shutdown. Already transferred pixels remain caller-owned
 * and must still be released. No callback invokes the Dart isolate. */
void rcode_visual_dispose(uint64_t visual);
char *rcode_visual_error(void);
void rcode_visual_string_free(char *text);

/* Synchronous bounded zatex layout. Returns owned JSON containing SVG glyph
 * outlines plus width/height/ascent, or NULL for invalid/incomplete input. */
char *rcode_math_svg(const char *request_json);

#ifdef __cplusplus
}
#endif
#endif
