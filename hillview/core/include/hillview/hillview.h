/* HillView core: host-agnostic image kernels + modulation, plain C ABI.
 *
 * No host SDK headers are used here. The Python app binds it via ctypes; host plugins
 * (OpenFX, AE SDK, FxPlug) link the same library. Everything is a pure function of its
 * inputs: no hidden state, so frames can be rendered in any order.
 *
 * Images are interleaved RGB, row-major, no padding: f32 in [0,1] or u8.
 * Return values: 0 on success, -1 invalid arguments, -2 out of memory.
 */
#ifndef HILLVIEW_H
#define HILLVIEW_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32)
#  define HV_API __declspec(dllexport)
#else
#  define HV_API __attribute__((visibility("default")))
#endif

#define HV_VERSION 1

enum { HV_QUANT_NONE = 0, HV_QUANT_POSTERIZE = 1, HV_QUANT_PALETTE = 2 };

/* One colour-assignment rule. Caller resolves everything time-dependent first
 * (modulated threshold, hue-shifted target, scaled amount). */
typedef struct {
    float src_lab[3];   /* source colour in OKLab */
    float tgt_rgb[3];   /* target colour, sRGB 0..1 */
    float threshold;    /* OKLab distance fully replaced (>= 0) */
    float softness;     /* extra distance over which the mask fades to 0 (> 0) */
    float amount;       /* 0..1 */
} HVRule;

/* Parameters for one frame, already resolved at the frame's time. */
typedef struct {
    int32_t quant_mode;         /* HV_QUANT_* */
    int32_t levels;             /* posterize levels per channel (>= 2) */
    const float *palette;       /* palette_n x 3 sRGB floats (HV_QUANT_PALETTE) */
    int32_t palette_n;
    float smooth_sigma;         /* blur before colour remap, px (< 0.4 disables) */
    float blur_sigma;           /* blur after colour remap, px (< 0.4 disables) */
    const HVRule *rules;
    int32_t n_rules;
} HVFrameParams;

HV_API int hv_version(void);

/* Full per-frame chain: quantize -> blur -> remap -> blur. `in` and `out` must not overlap.
 * threads <= 0 means "all hardware threads". Thread-safe: no global state. */
HV_API int hv_process_f32(const float *in, float *out, int w, int h, const HVFrameParams *p, int threads);
HV_API int hv_process_u8(const uint8_t *in, uint8_t *out, int w, int h, const HVFrameParams *p, int threads);

/* out = (a*(256-w) + b*w + 128) >> 8 per byte, w in 0..256: frame interpolation between two processed keyframes. */
HV_API int hv_blend_u8(const uint8_t *a, const uint8_t *b, uint8_t *out, int w, int h, int weight256, int threads);

/* ---- modulation (deterministic: pure functions of time and seed) ---- */
enum { HV_SRC_LFO = 0, HV_SRC_NOISE = 1, HV_SRC_STEPS = 2 };
enum { HV_LFO_SINE = 0, HV_LFO_SAW = 1, HV_LFO_TRI = 2, HV_LFO_SQUARE = 3 };

HV_API double hv_hash01(int64_t n, int64_t seed);
/* Value in [0,1]. `shape`/`phase`/`duty` apply to LFOs only. */
HV_API double hv_mod_source(int type, int shape, double hz, double phase, double duty, int64_t seed, double t);

#ifdef __cplusplus
}
#endif
#endif
