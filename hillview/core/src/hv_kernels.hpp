#pragma once
#include <cstdint>

#include "hillview/hillview.h"

namespace hv {
void posterize(const float *in, float *out, int w, int h, int levels, int threads);
void apply_palette(const float *in, float *out, int w, int h, const float *pal, int k, int threads);
// Blur in -> out using `tmp` (w*h*3 floats) as scratch. Does nothing if sigma < 0.4. Returns 0, or -2 if out of memory.
int blur(const float *in, float *out, float *tmp, int w, int h, float sigma, int threads);
void remap(const float *in, float *out, int w, int h, const HVRule *rules, int nr, int threads);
void u8_to_f32(const uint8_t *in, float *out, int w, int h, int threads);
void f32_to_u8(const float *in, uint8_t *out, int w, int h, int threads);
void blend_u8(const uint8_t *a, const uint8_t *b, uint8_t *out, int w, int h, int weight256, int threads);
}  // namespace hv
