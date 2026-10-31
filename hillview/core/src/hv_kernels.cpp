// Image kernels: posterize, palette, blur (separable running-sum box x3), colour remap.
#include "hv_kernels.hpp"

#include <cmath>
#include <cstring>
#include <utility>

#include "hv_color.hpp"
#include "hv_util.hpp"

namespace hv {

namespace {
struct LutHolder {
    float v[kLutN + 1];
    LutHolder() {
        for (int i = 0; i <= kLutN; ++i) {
            double x = (double)i / kLutN;
            v[i] = (float)(x <= 0.04045 ? x / 12.92 : std::pow((x + 0.055) / 1.055, 2.4));
        }
    }
};
const LutHolder g_lut;   // constructed at load time
}  // namespace
const float *const kSrgbLut = g_lut.v;

// ------------------------------------------------------------------ quantize
HV_MV static void posterize_span(const float *in, float *out, size_t n, float levels_m1, float inv) {
    const float magic = 12582912.0f;   // 1.5 * 2^23: (x + magic) - magic rounds to nearest-even for |x| < 2^22
    for (size_t i = 0; i < n; ++i) { float v = in[i] * levels_m1; out[i] = ((v + magic) - magic) * inv; }
}

void posterize(const float *in, float *out, int w, int h, int levels, int threads) {
    float m1 = (float)(levels - 1), inv = 1.0f / m1;
    parallel_for(h, threads, 16, [&](int y0, int y1) {
        posterize_span(in + (size_t)y0 * w * 3, out + (size_t)y0 * w * 3, (size_t)(y1 - y0) * w * 3, m1, inv);
    });
}

HV_MV static void palette_span(const float *in, float *out, int npx, const float *pal, const float *pn, int k) {
    for (int i = 0; i < npx; ++i) {
        float r = in[3 * i], g = in[3 * i + 1], b = in[3 * i + 2];
        float best = pn[0] - 2.f * (r * pal[0] + g * pal[1] + b * pal[2]);   // |c|^2 - 2 x.c  (|x|^2 constant)
        int bi = 0;
        for (int j = 1; j < k; ++j) {
            float s = pn[j] - 2.f * (r * pal[3 * j] + g * pal[3 * j + 1] + b * pal[3 * j + 2]);
            if (s < best) { best = s; bi = j; }
        }
        out[3 * i] = pal[3 * bi]; out[3 * i + 1] = pal[3 * bi + 1]; out[3 * i + 2] = pal[3 * bi + 2];
    }
}

void apply_palette(const float *in, float *out, int w, int h, const float *pal, int k, int threads) {
    float pn[256];
    if (k > 256) k = 256;
    for (int j = 0; j < k; ++j) pn[j] = pal[3 * j] * pal[3 * j] + pal[3 * j + 1] * pal[3 * j + 1] + pal[3 * j + 2] * pal[3 * j + 2];
    parallel_for(h, threads, 16, [&](int y0, int y1) {
        palette_span(in + (size_t)y0 * w * 3, out + (size_t)y0 * w * 3, (y1 - y0) * w, pal, pn, k);
    });
}

// ------------------------------------------------------------------ blur
// out[j] = mean(in[j .. j+k]) for j in [0, n-k): running sum over interleaved RGB samples.
static void box_valid_rgb(const float *in, float *out, int n, int k) {
    int m = n - k;
    float scale = 1.0f / (float)(k + 1);
    float a0 = 0, a1 = 0, a2 = 0;
    for (int i = 0; i <= k; ++i) { a0 += in[3 * i]; a1 += in[3 * i + 1]; a2 += in[3 * i + 2]; }
    out[0] = a0 * scale; out[1] = a1 * scale; out[2] = a2 * scale;
    for (int j = 1; j < m; ++j) {
        const float *add = in + 3 * (j + k), *sub = in + 3 * (j - 1);
        a0 += add[0] - sub[0]; a1 += add[1] - sub[1]; a2 += add[2] - sub[2];
        out[3 * j] = a0 * scale; out[3 * j + 1] = a1 * scale; out[3 * j + 2] = a2 * scale;
    }
}

// Horizontal blur: each row is edge-padded by 3r once, then three box passes.
static bool hblur(const float *src, float *dst, int w, int h, int r, int threads) {
    std::atomic<bool> oom{false};
    parallel_for(h, threads, 8, [&](int y0, int y1) {
        int pw = w + 6 * r;
        FBuf A((size_t)pw * 3), B((size_t)pw * 3);
        if (!A.ok() || !B.ok()) { oom = true; return; }
        for (int y = y0; y < y1; ++y) {
            const float *row = src + (size_t)y * w * 3;
            for (int i = 0; i < 3 * r; ++i) std::memcpy(A.get() + 3 * i, row, 3 * sizeof(float));
            std::memcpy(A.get() + 3 * 3 * r, row, (size_t)w * 3 * sizeof(float));
            for (int i = 0; i < 3 * r; ++i) std::memcpy(A.get() + 3 * (3 * r + w + i), row + 3 * (w - 1), 3 * sizeof(float));
            float *in = A.get(), *out = B.get();
            int n = pw;
            for (int p = 0; p < 3; ++p) { box_valid_rgb(in, out, n, 2 * r); std::swap(in, out); n -= 2 * r; }
            std::memcpy(dst + (size_t)y * w * 3, in, (size_t)w * 3 * sizeof(float));
        }
    });
    return !oom;
}

// acc += add - sub; out = acc * scale   (vector over a band of columns)
HV_MV static void run_step(float *acc, const float *add, const float *sub, float *out, int n, float scale) {
    for (int i = 0; i < n; ++i) { acc[i] += add[i] - sub[i]; out[i] = acc[i] * scale; }
}
HV_MV static void run_init(float *acc, const float *rows, int stride, int count, int n) {
    for (int i = 0; i < n; ++i) acc[i] = 0.f;
    for (int c = 0; c < count; ++c) { const float *p = rows + (size_t)c * stride; for (int i = 0; i < n; ++i) acc[i] += p[i]; }
}

// Vertical blur in column bands: a band's padded copy stays in L2 while the three passes run.
static bool vblur(const float *src, float *dst, int w, int h, int r, int threads) {
    const int BW = 16;                         // pixels per band
    int nb = (w + BW - 1) / BW;
    std::atomic<bool> oom{false};
    parallel_for(nb, threads, 1, [&](int b0, int b1) {
        int ph = h + 6 * r, k = 2 * r;
        FBuf A((size_t)ph * BW * 3), B((size_t)ph * BW * 3);
        if (!A.ok() || !B.ok()) { oom = true; return; }
        float acc[BW * 3];
        for (int b = b0; b < b1; ++b) {
            int x0 = b * BW, bwpx = std::min(BW, w - x0), n = bwpx * 3;
            for (int i = 0; i < ph; ++i) {
                int sy = std::min(std::max(i - 3 * r, 0), h - 1);
                std::memcpy(A.get() + (size_t)i * n, src + ((size_t)sy * w + x0) * 3, (size_t)n * sizeof(float));
            }
            float *in = A.get(), *out = B.get();
            int rows = ph;
            float scale = 1.0f / (float)(k + 1);
            for (int p = 0; p < 3; ++p) {
                int m = rows - k;
                run_init(acc, in, n, k + 1, n);
                for (int i = 0; i < n; ++i) out[i] = acc[i] * scale;
                for (int j = 1; j < m; ++j) run_step(acc, in + (size_t)(j + k) * n, in + (size_t)(j - 1) * n, out + (size_t)j * n, n, scale);
                std::swap(in, out);
                rows = m;
            }
            for (int y = 0; y < h; ++y) std::memcpy(dst + ((size_t)y * w + x0) * 3, in + (size_t)y * n, (size_t)n * sizeof(float));
        }
    });
    return !oom;
}

int blur(const float *in, float *out, float *tmp, int w, int h, float sigma, int threads) {
    if (sigma < 0.4f) return 0;
    int r = (int)std::nearbyint((std::sqrt(4.0 * sigma * sigma + 1.0) - 1.0) / 2.0);   // 3 boxes: var = 3(w^2-1)/12
    if (r < 1) r = 1;
    if (!vblur(in, tmp, w, h, r, threads)) return -2;
    if (!hblur(tmp, out, w, h, r, threads)) return -2;
    return 0;
}

// ------------------------------------------------------------------ colour remap
// Chunked structure-of-arrays so every inner loop is simple enough to auto-vectorise (the per-pixel
// version with interleaved RGB, a table gather and a rule loop ran ~5x slower). Chunks stay in L1.
constexpr int kChunk = 128;
static inline float clamp01(float v) { return std::min(std::max(v, 0.f), 1.f); }   // min/max, not ?: -> vectorises

HV_MV static void remap_chunk(const float *in, float *out, int n, const HVRule *rules, int nr, const float *lut_v) {
    float lr[kChunk], lg[kChunk], lb[kChunk], L[kChunk], A[kChunk], B[kChunk], o0[kChunk], o1[kChunk], o2[kChunk];
    const float kN = (float)kLutN;
    for (int i = 0; i < n; ++i) {                       // deinterleave + sRGB decode (table lookup)
        float r = in[3 * i], g = in[3 * i + 1], b = in[3 * i + 2];
        o0[i] = r; o1[i] = g; o2[i] = b;
        float x = clamp01(r) * kN + 0.5f, y = clamp01(g) * kN + 0.5f, z = clamp01(b) * kN + 0.5f;
        lr[i] = lut_v[(int)x]; lg[i] = lut_v[(int)y]; lb[i] = lut_v[(int)z];
    }
    for (int i = 0; i < n; ++i) {                       // linear RGB -> OKLab
        float l = fast_cbrt(0.4122214708f * lr[i] + 0.5363325363f * lg[i] + 0.0514459929f * lb[i]);
        float m = fast_cbrt(0.2119034982f * lr[i] + 0.6806995451f * lg[i] + 0.1073969566f * lb[i]);
        float s = fast_cbrt(0.0883024619f * lr[i] + 0.2817188376f * lg[i] + 0.6299787005f * lb[i]);
        L[i] = 0.2104542553f * l + 0.7936177850f * m - 0.0040720468f * s;
        A[i] = 1.9779984951f * l - 2.4285922050f * m + 0.4505937099f * s;
        B[i] = 0.0259040371f * l + 0.7827717662f * m - 0.8086757660f * s;
    }
    for (int q = 0; q < nr; ++q) {                      // masks come from the input colour: rules never chain
        const HVRule u = rules[q];
        const float inv_soft = 1.0f / u.softness;
        for (int i = 0; i < n; ++i) {
            float dl = L[i] - u.src_lab[0], da = A[i] - u.src_lab[1], db = B[i] - u.src_lab[2];
            float t = (std::sqrt(dl * dl + da * da + db * db) - u.threshold) * inv_soft;
            t = clamp01(t);
            float m = u.amount * (1.f - t * t * (3.f - 2.f * t));
            o0[i] += m * (u.tgt_rgb[0] - o0[i]);
            o1[i] += m * (u.tgt_rgb[1] - o1[i]);
            o2[i] += m * (u.tgt_rgb[2] - o2[i]);
        }
    }
    for (int i = 0; i < n; ++i) { out[3 * i] = o0[i]; out[3 * i + 1] = o1[i]; out[3 * i + 2] = o2[i]; }
}

static void remap_span(const float *in, float *out, int npx, const HVRule *rules, int nr, const float *lut_v) {
    for (int i = 0; i < npx; i += kChunk) remap_chunk(in + 3 * (size_t)i, out + 3 * (size_t)i, std::min(kChunk, npx - i), rules, nr, lut_v);
}

void remap(const float *in, float *out, int w, int h, const HVRule *rules, int nr, int threads) {
    const float *lv = kSrgbLut;
    parallel_for(h, threads, 16, [&](int y0, int y1) {
        remap_span(in + (size_t)y0 * w * 3, out + (size_t)y0 * w * 3, (y1 - y0) * w, rules, nr, lv);
    });
}

// ------------------------------------------------------------------ u8 <-> f32
HV_MV static void u8_to_f32_span(const uint8_t *in, float *out, size_t n) {
    const float s = 1.0f / 255.0f;
    for (size_t i = 0; i < n; ++i) out[i] = (float)in[i] * s;
}
HV_MV static void f32_to_u8_span(const float *in, uint8_t *out, size_t n) {
    for (size_t i = 0; i < n; ++i) {
        float v = in[i] * 255.0f + 0.5f;
        v = std::min(std::max(v, 0.f), 255.f);
        out[i] = (uint8_t)v;
    }
}
void u8_to_f32(const uint8_t *in, float *out, int w, int h, int threads) {
    parallel_for(h, threads, 32, [&](int y0, int y1) { u8_to_f32_span(in + (size_t)y0 * w * 3, out + (size_t)y0 * w * 3, (size_t)(y1 - y0) * w * 3); });
}
void f32_to_u8(const float *in, uint8_t *out, int w, int h, int threads) {
    parallel_for(h, threads, 32, [&](int y0, int y1) { f32_to_u8_span(in + (size_t)y0 * w * 3, out + (size_t)y0 * w * 3, (size_t)(y1 - y0) * w * 3); });
}

HV_MV static void blend_span(const uint8_t *a, const uint8_t *b, uint8_t *out, size_t n, uint32_t w) {
    const uint32_t iw = 256 - w;
    for (size_t i = 0; i < n; ++i) out[i] = (uint8_t)((a[i] * iw + b[i] * w + 128u) >> 8);
}
void blend_u8(const uint8_t *a, const uint8_t *b, uint8_t *out, int w, int h, int weight256, int threads) {
    parallel_for(h, threads, 32, [&](int y0, int y1) {
        size_t o = (size_t)y0 * w * 3;
        blend_span(a + o, b + o, out + o, (size_t)(y1 - y0) * w * 3, (uint32_t)weight256);
    });
}

}  // namespace hv
