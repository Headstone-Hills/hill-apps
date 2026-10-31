// sRGB <-> OKLab, matching hillview/color.py (12-bit LUT decode, same matrices).
#pragma once
#include <cmath>
#include <cstdint>
#include <cstring>

namespace hv {

constexpr int kLutN = 4096;

// sRGB decode table, filled when the library loads (a function-local static would need the C++ runtime's init guard).
extern const float *const kSrgbLut;   // kLutN + 1 entries; defined in hv_kernels.cpp

// Cube root for x >= 0: bit-trick seed + two Halley steps (rel. error ~1e-7); vectorises, unlike cbrtf.
inline float fast_cbrt(float x) {
    uint32_t i;
    std::memcpy(&i, &x, 4);
    i = i / 3u + 0x2a514067u;
    float y;
    std::memcpy(&y, &i, 4);
    for (int k = 0; k < 2; ++k) {
        float y3 = y * y * y;
        y = y * (y3 + 2.f * x) / (2.f * y3 + x);
    }
    return y;
}

// r,g,b in [0,1] -> OKLab
inline void rgb_to_oklab(const float *lut_v, float r, float g, float b, float &L, float &A, float &B) {
    auto dec = [&](float c) {
        c = c < 0.f ? 0.f : (c > 1.f ? 1.f : c);
        float idx = c * (float)kLutN;
        idx += 0.5f;
        return lut_v[(int)idx];
    };
    float lr = dec(r), lg = dec(g), lb = dec(b);
    float l = 0.4122214708f * lr + 0.5363325363f * lg + 0.0514459929f * lb;
    float m = 0.2119034982f * lr + 0.6806995451f * lg + 0.1073969566f * lb;
    float s = 0.0883024619f * lr + 0.2817188376f * lg + 0.6299787005f * lb;
    l = fast_cbrt(l); m = fast_cbrt(m); s = fast_cbrt(s);
    L = 0.2104542553f * l + 0.7936177850f * m - 0.0040720468f * s;
    A = 1.9779984951f * l - 2.4285922050f * m + 0.4505937099f * s;
    B = 0.0259040371f * l + 0.7827717662f * m - 0.8086757660f * s;
}

}  // namespace hv
