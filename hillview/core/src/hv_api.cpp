#include <cmath>
#include <cstring>

#include "hillview/hillview.h"
#include "hv_kernels.hpp"
#include "hv_util.hpp"

namespace {

// Runs quantize -> blur -> remap -> blur between `in` and `out` using thread-local scratch.
// Each stage writes to a buffer distinct from its input; the last stage writes straight to `out`.
int run_chain(const float *in, float *out, int w, int h, const HVFrameParams &p, int threads, int first_slot) {
    const size_t n = (size_t)w * h * 3;
    const bool quant = (p.quant_mode == HV_QUANT_POSTERIZE && p.levels >= 2) ||
                       (p.quant_mode == HV_QUANT_PALETTE && p.palette && p.palette_n >= 1);
    const bool smooth = p.smooth_sigma >= 0.4f, remap = p.n_rules > 0 && p.rules, blur2 = p.blur_sigma >= 0.4f;
    int remaining = (int)quant + (int)smooth + (int)remap + (int)blur2;
    if (remaining == 0) { std::memcpy(out, in, n * sizeof(float)); return 0; }

    float *bufs[3] = {hv::scratch(first_slot, n), hv::scratch(first_slot + 1, n), hv::scratch(first_slot + 2, n)};
    auto pick = [&](const float *a, const float *b) -> float * {
        for (float *x : bufs) if (x != a && x != b) return x;
        return bufs[0];
    };
    const float *cur = in;
    auto dest = [&](bool last) { return last ? out : pick(cur, nullptr); };

    if (quant) {
        float *d = dest(--remaining == 0);
        if (p.quant_mode == HV_QUANT_POSTERIZE) hv::posterize(cur, d, w, h, p.levels, threads);
        else hv::apply_palette(cur, d, w, h, p.palette, p.palette_n, threads);
        cur = d;
    }
    auto do_blur = [&](float sigma) {
        float *d = dest(--remaining == 0);
        hv::blur(cur, d, pick(cur, d), w, h, sigma, threads);
        cur = d;
    };
    if (smooth) do_blur(p.smooth_sigma);
    if (remap) {
        float *d = dest(--remaining == 0);
        hv::remap(cur, d, w, h, p.rules, p.n_rules, threads);
        cur = d;
    }
    if (blur2) do_blur(p.blur_sigma);
    return 0;
}

inline bool bad(int w, int h, const void *a, const void *b, const HVFrameParams *p) {
    return w <= 0 || h <= 0 || !a || !b || !p || (const void *)a == (const void *)b;
}

}  // namespace

extern "C" {

int hv_version(void) { return HV_VERSION; }

int hv_process_f32(const float *in, float *out, int w, int h, const HVFrameParams *p, int threads) {
    if (bad(w, h, in, out, p)) return -1;
    return run_chain(in, out, w, h, *p, threads, 0);
}

int hv_process_u8(const uint8_t *in, uint8_t *out, int w, int h, const HVFrameParams *p, int threads) {
    if (bad(w, h, in, out, p)) return -1;
    const size_t n = (size_t)w * h * 3;
    float *fin = hv::scratch(3, n), *fout = hv::scratch(4, n);   // slots 0-2 belong to run_chain
    hv::u8_to_f32(in, fin, w, h, threads);
    int rc = run_chain(fin, fout, w, h, *p, threads, 0);
    if (rc == 0) hv::f32_to_u8(fout, out, w, h, threads);
    return rc;
}

int hv_blend_u8(const uint8_t *a, const uint8_t *b, uint8_t *out, int w, int h, int weight256, int threads) {
    if (w <= 0 || h <= 0 || !a || !b || !out || weight256 < 0 || weight256 > 256) return -1;
    hv::blend_u8(a, b, out, w, h, weight256, threads);
    return 0;
}

// ---------------------------------------------------------------- modulation
double hv_hash01(int64_t n, int64_t seed) {
    uint64_t z = (uint64_t)n * 0x9E3779B97F4A7C15ULL + (uint64_t)seed * 0xD1B54A32D192ED03ULL + 0x632BE59BD9B4E019ULL;
    z = (z ^ (z >> 30)) * 0xBF58476D1CE4E5B9ULL;
    z = (z ^ (z >> 27)) * 0x94D049BB133111EBULL;
    z ^= z >> 31;
    return (double)(z >> 11) / 9007199254740992.0;   // 2^53
}

double hv_mod_source(int type, int shape, double hz, double phase, double duty, int64_t seed, double t) {
    const double PI = 3.14159265358979323846;
    if (type == HV_SRC_LFO) {
        double p = std::fmod(t * hz + phase, 1.0);
        if (p < 0) p += 1.0;                                  // Python's % is never negative
        switch (shape) {
            case HV_LFO_SINE: return 0.5 - 0.5 * std::cos(2 * PI * p);
            case HV_LFO_SAW: return p;
            case HV_LFO_TRI: return 1.0 - std::fabs(2 * p - 1);
            case HV_LFO_SQUARE: return p < duty ? 1.0 : 0.0;
        }
        return 0.0;
    }
    if (type == HV_SRC_NOISE) {
        double x = t * hz, fl = std::floor(x), f = x - fl, s = f * f * (3 - 2 * f);
        return hv_hash01((int64_t)fl, seed) * (1 - s) + hv_hash01((int64_t)fl + 1, seed) * s;
    }
    if (type == HV_SRC_STEPS) return hv_hash01((int64_t)std::floor(t * hz), seed);
    return 0.0;
}

}  // extern "C"
