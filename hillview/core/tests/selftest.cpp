// Standalone self-test for the native core (no Python): runs natively, or under qemu on other architectures.
//  - kernels vs naive double-precision references on many sizes/radii
//  - kernels vs golden output from the numpy reference (golden.inc, see gen_golden.py)
//  - modulation vs golden values from the Python definitions
//  - bitwise determinism across thread counts, argument validation
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <vector>

#include "hillview/hillview.h"
#include "golden.inc"

static int g_fail = 0, g_checks = 0;
#define CHECK(cond, ...) do { ++g_checks; if (!(cond)) { ++g_fail; std::printf("FAIL %s:%d: ", __FILE__, __LINE__); std::printf(__VA_ARGS__); std::printf("\n"); } } while (0)

static std::vector<uint8_t> lcg_image(int w, int h, uint32_t s = 12345) {
    std::vector<uint8_t> v((size_t)w * h * 3);
    for (auto &b : v) { s = s * 1664525u + 1013904223u; b = (uint8_t)(s >> 24); }
    for (int y = 0; y < h / 3; ++y) for (int x = 0; x < w / 2; ++x) { uint8_t *p = &v[((size_t)y * w + x) * 3]; p[0] = 200; p[1] = 30; p[2] = 40; }
    return v;
}

static HVFrameParams params(int quant, int levels, const float *pal, int npal, float smooth, float blur, const HVRule *rules, int nr) {
    HVFrameParams p; std::memset(&p, 0, sizeof p);
    p.quant_mode = quant; p.levels = levels; p.palette = pal; p.palette_n = npal;
    p.smooth_sigma = smooth; p.blur_sigma = blur; p.rules = rules; p.n_rules = nr;
    return p;
}

static std::vector<HVRule> golden_rules() {
    std::vector<HVRule> r;
    for (auto &g : G_RULES) { HVRule u; for (int i = 0; i < 3; ++i) { u.src_lab[i] = g[i]; u.tgt_rgb[i] = g[3 + i]; } u.threshold = g[6]; u.softness = g[7]; u.amount = g[8]; r.push_back(u); }
    return r;
}

static int max_diff(const uint8_t *a, const uint8_t *b, size_t n, size_t *n_off = nullptr) {
    int m = 0; size_t off = 0;
    for (size_t i = 0; i < n; ++i) { int d = std::abs((int)a[i] - (int)b[i]); if (d > m) m = d; off += d != 0; }
    if (n_off) *n_off = off;
    return m;
}

// ---- naive references (double precision)
static std::vector<double> ref_blur(const std::vector<float> &in, int w, int h, float sigma) {
    int r = std::max(1, (int)std::nearbyint((std::sqrt(4.0 * sigma * sigma + 1.0) - 1.0) / 2.0));
    std::vector<double> cur(in.begin(), in.end()), nxt(cur.size());
    for (int axis = 0; axis < 2; ++axis)                       // vertical then horizontal, three box passes each, edge-extended
        for (int pass = 0; pass < 3; ++pass) {
            for (int y = 0; y < h; ++y) for (int x = 0; x < w; ++x) for (int c = 0; c < 3; ++c) {
                double s = 0;
                for (int k = -r; k <= r; ++k) {
                    int yy = axis == 0 ? std::min(std::max(y + k, 0), h - 1) : y, xx = axis == 1 ? std::min(std::max(x + k, 0), w - 1) : x;
                    s += cur[((size_t)yy * w + xx) * 3 + c];
                }
                nxt[((size_t)y * w + x) * 3 + c] = s / (2 * r + 1);
            }
            cur.swap(nxt);
        }
    return cur;
}
// NB: iterating 3 box passes on the clamped image equals padding once by 3r only when radii compose; the kernel pads once
// (infinite edge extension), and clamping the *intermediate* each pass differs at borders. So compare interiors only.

static void test_blur_vs_reference() {
    const int sizes[][2] = {{1, 1}, {3, 5}, {16, 16}, {17, 33}, {40, 9}, {64, 48}};
    const float sigmas[] = {0.5f, 1.5f, 3.f, 6.f};
    for (auto &sz : sizes) for (float sg : sigmas) {
        int w = sz[0], h = sz[1];
        auto img = lcg_image(w, h, 7 + w * 31 + h);
        std::vector<float> f(img.size()), out(img.size());
        for (size_t i = 0; i < f.size(); ++i) f[i] = img[i] / 255.f;
        HVFrameParams p = params(0, 2, nullptr, 0, sg, 0, nullptr, 0);
        CHECK(hv_process_f32(f.data(), out.data(), w, h, &p, 1) == 0, "rc");
        auto ref = ref_blur(f, w, h, sg);
        int r = std::max(1, (int)std::nearbyint((std::sqrt(4.0 * sg * sg + 1.0) - 1.0) / 2.0)), m = 3 * r + 1;
        double worst = 0;
        for (int y = m; y < h - m; ++y) for (int x = m; x < w - m; ++x) for (int c = 0; c < 3; ++c)
            worst = std::max(worst, std::fabs(out[((size_t)y * w + x) * 3 + c] - ref[((size_t)y * w + x) * 3 + c]));
        CHECK(worst < 1e-4, "blur %dx%d sigma %.1f interior error %.2e", w, h, sg, worst);
        // constant image stays constant everywhere, borders included
        std::vector<float> cst(f.size(), 0.37f);
        hv_process_f32(cst.data(), out.data(), w, h, &p, 1);
        double dev = 0; for (float v : out) dev = std::max(dev, (double)std::fabs(v - 0.37f));
        CHECK(dev < 1e-5, "constant image drifted by %.2e at %dx%d", dev, w, h);
    }
}

static void test_palette_vs_bruteforce() {
    const int w = 29, h = 17;
    auto img = lcg_image(w, h);
    std::vector<float> f(img.size()), out(img.size());
    for (size_t i = 0; i < f.size(); ++i) f[i] = img[i] / 255.f;
    HVFrameParams p = params(HV_QUANT_PALETTE, 2, G_PALETTE, 5, 0, 0, nullptr, 0);
    CHECK(hv_process_f32(f.data(), out.data(), w, h, &p, 2) == 0, "rc");
    int bad = 0;
    for (int i = 0; i < w * h; ++i) {
        int best = 0; double bd = 1e9;
        for (int j = 0; j < 5; ++j) { double d = 0; for (int c = 0; c < 3; ++c) { double e = f[3 * i + c] - G_PALETTE[3 * j + c]; d += e * e; } if (d < bd) { bd = d; best = j; } }
        for (int c = 0; c < 3; ++c) bad += out[3 * i + c] != G_PALETTE[3 * best + c];
    }
    CHECK(bad == 0, "%d palette channel mismatches vs brute force", bad);
}

static void test_posterize_ties() {
    // exact half-way values must round to even, like numpy.round: 3 levels -> x*2 in {0.5,1.5} => 0 and 2
    float in[6] = {0.25f, 0.25f, 0.25f, 0.75f, 0.75f, 0.75f}, out[6];
    HVFrameParams p = params(HV_QUANT_POSTERIZE, 3, nullptr, 0, 0, 0, nullptr, 0);
    CHECK(hv_process_f32(in, out, 2, 1, &p, 1) == 0, "rc");
    CHECK(out[0] == 0.f && out[3] == 1.f, "ties: 0.25 -> %.3f (want 0), 0.75 -> %.3f (want 1)", out[0], out[3]);
}

static void test_golden() {
    const int w = GW, h = GH;
    auto img = lcg_image(w, h);
    auto rules = golden_rules();
    std::vector<uint8_t> out(img.size());
    size_t off;
    struct Case { const char *name; HVFrameParams p; const uint8_t *want; int tol; } cases[] = {
        {"chain A (posterize,blur,remap x2,blur)", params(HV_QUANT_POSTERIZE, 4, nullptr, 0, 3.f, 2.f, rules.data(), 2), G_CHAIN_A, 1},
        {"chain B (palette,blur)", params(HV_QUANT_PALETTE, 2, G_PALETTE, 5, 0, 1.5f, nullptr, 0), G_CHAIN_B, 1},
        {"remap only", params(0, 2, nullptr, 0, 0, 0, rules.data(), 2), G_REMAP_ONLY, 1},
    };
    for (auto &c : cases) {
        CHECK(hv_process_u8(img.data(), out.data(), w, h, &c.p, 1) == 0, "rc");
        int d = max_diff(out.data(), c.want, out.size(), &off);
        CHECK(d <= c.tol, "golden '%s': max diff %d levels (%zu of %zu bytes differ)", c.name, d, off, out.size());
        std::printf("  golden %-42s max diff %d level(s), %zu/%zu bytes differ\n", c.name, d, off, out.size());
    }
}

static void test_threads_bitwise_identical() {
    const int w = 83, h = 61;
    auto img = lcg_image(w, h, 99);
    auto rules = golden_rules();
    HVFrameParams p = params(HV_QUANT_POSTERIZE, 5, nullptr, 0, 2.5f, 1.5f, rules.data(), 2);
    std::vector<uint8_t> ref(img.size()), out(img.size());
    hv_process_u8(img.data(), ref.data(), w, h, &p, 1);
    for (int t : {2, 3, 7}) { hv_process_u8(img.data(), out.data(), w, h, &p, t); CHECK(std::memcmp(ref.data(), out.data(), ref.size()) == 0, "threads=%d differs from threads=1", t); }
}

static void test_blend() {
    const int w = 13, h = 7;
    auto a = lcg_image(w, h, 1), b = lcg_image(w, h, 2);
    std::vector<uint8_t> out(a.size());
    for (int wt : {0, 1, 100, 128, 255, 256}) {
        CHECK(hv_blend_u8(a.data(), b.data(), out.data(), w, h, wt, 3) == 0, "rc");
        bool ok = true;
        for (size_t i = 0; i < a.size(); ++i) ok &= out[i] == (uint8_t)((a[i] * (256 - wt) + b[i] * wt + 128) >> 8);
        CHECK(ok, "blend weight %d mismatch", wt);
    }
}

static void test_args() {
    float buf[12] = {0}, out[12];
    HVFrameParams p = params(0, 2, nullptr, 0, 0, 0, nullptr, 0);
    CHECK(hv_process_f32(buf, buf, 2, 2, &p, 1) == -1, "aliased");
    CHECK(hv_process_f32(nullptr, out, 2, 2, &p, 1) == -1, "null in");
    CHECK(hv_process_f32(buf, out, 0, 2, &p, 1) == -1, "w=0");
    CHECK(hv_process_f32(buf, out, 2, 2, nullptr, 1) == -1, "null params");
    uint8_t u[12] = {0};
    CHECK(hv_blend_u8(u, u, u, 2, 2, 257, 1) == -1 && hv_blend_u8(u, u, u, 2, 2, -1, 1) == -1, "blend weight range");
    CHECK(hv_version() == HV_VERSION, "version");
}

static void test_modulation() {
    for (auto &g : G_HASH) CHECK(hv_hash01((int64_t)g[0], (int64_t)g[1]) == g[2], "hash(%g,%g)", g[0], g[1]);
    double worst = 0;
    for (auto &g : G_MOD) worst = std::max(worst, std::fabs(hv_mod_source((int)g[0], (int)g[1], g[2], g[3], g[4], (int64_t)g[5], g[6]) - g[7]));
    CHECK(worst < 1e-12, "modulation sources deviate from Python by %.2e", worst);
}

int main() {
#if defined(__aarch64__)
    const char *arch = "aarch64";
#elif defined(__x86_64__)
    const char *arch = "x86_64";
#else
    const char *arch = "other";
#endif
    std::printf("HillView core self-test (arch %s, core v%d)\n", arch, hv_version());
    test_blur_vs_reference(); test_palette_vs_bruteforce(); test_posterize_ties(); test_golden();
    test_threads_bitwise_identical(); test_blend(); test_args(); test_modulation();
    std::printf("%d checks, %d failed\n", g_checks, g_fail);
    return g_fail ? 1 : 0;
}
