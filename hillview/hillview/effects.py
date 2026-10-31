"""Stateless image operators. All take/return float32 RGB in [0,1], shape (H,W,3)."""
import numpy as np
from .color import (rgb_to_oklab, rgb_to_oklab_fast, oklab_to_rgb, parse_color, rotate_hue_lab)


def posterize(img, levels):
    levels = max(2, int(round(levels)))
    return np.round(img * (levels - 1)) / (levels - 1)


def kmeans_palette(img, k, seed=0, iters=8):
    """Fit a k-colour palette (k-means++ on a 4096-pixel sample). Deterministic for a given seed.
    `img` may be float [0,1] or uint8 (only the sample is converted). Returns (k,3) float32."""
    k = max(2, int(round(k)))
    flat = img.reshape(-1, 3)
    rng = np.random.default_rng(seed)
    sub = flat[rng.integers(0, len(flat), size=min(4096, len(flat)))]
    if sub.dtype == np.uint8:
        sub = sub.astype(np.float32) * np.float32(1 / 255)
    cent = [sub[rng.integers(len(sub))]]
    for _ in range(k - 1):  # k-means++ init
        d = np.min([((sub - c) ** 2).sum(1) for c in cent], axis=0)
        s = d.sum()
        cent.append(sub[rng.choice(len(sub), p=d / s) if s > 0 else rng.integers(len(sub))])
    cent = np.array(cent, np.float32)
    for _ in range(iters):
        idx = _nearest(sub, cent)
        for j in range(k):
            m = idx == j
            if m.any():
                cent[j] = sub[m].mean(0)
    return cent


def kmeans_quantize(img, k, seed=0, iters=8):
    """Palette quantize to k colours (numpy reference path)."""
    cent = kmeans_palette(img, k, seed, iters)
    h, w, _ = img.shape
    flat = img.reshape(-1, 3)
    out = np.empty_like(flat)
    for s in range(0, len(flat), 262144):  # chunked to bound memory
        out[s:s + 262144] = cent[_nearest(flat[s:s + 262144], cent)]
    return out.reshape(h, w, 3)


def _nearest(x, cent):
    """Index of the nearest centroid per row. argmin ||x-c||^2 = argmin(||c||^2 - 2 x.c): one BLAS matmul."""
    score = x @ (-2.0 * cent.T)
    score += (cent * cent).sum(1)
    return score.argmin(1)


def _box_rows(x, r):
    """Box blur of radius r down the rows of a 2-D float32 array, "valid" mode (output is 2r rows
    shorter). Running sum: each step adds the row entering the window and subtracts the one
    leaving, vectorised across the whole row. ~6x faster than np.cumsum along axis 0."""
    k = 2 * r
    m = x.shape[0] - k
    out = np.empty((m, x.shape[1]), np.float32)
    acc = x[:k + 1].sum(0, dtype=np.float32)
    scale = np.float32(1.0 / (k + 1))
    np.multiply(acc, scale, out=out[0])
    for j in range(1, m):
        acc += x[j + k]
        acc -= x[j - 1]
        np.multiply(acc, scale, out=out[j])
    return out


def _gauss3(x, sigma):
    """Approximate Gaussian on (H,W,3): three box passes per axis. Edge-padded once per axis (3r px)
    so the result equals blurring the infinitely edge-extended image."""
    r = max(1, int(round((np.sqrt(4 * sigma * sigma + 1) - 1) / 2)))  # 3 boxes: var = 3*(w^2-1)/12
    for t in (0, 1):
        if t:                                   # switch axes with one contiguous transpose
            x = np.ascontiguousarray(x.transpose(1, 0, 2))
        n, m, c = x.shape
        x = np.pad(x, ((3 * r, 3 * r), (0, 0), (0, 0)), mode="edge").reshape(n + 6 * r, m * c)
        for _ in range(3):
            x = _box_rows(x, r)
        x = x.reshape(n, m, c)
    return np.ascontiguousarray(x.transpose(1, 0, 2))


def _reduce(x, f):
    """Block-mean downscale of (H,W,3) by integer f (edge-pads to a multiple of f)."""
    h, w, c = x.shape
    ph, pw = -h % f, -w % f
    if ph or pw:
        x = np.pad(x, ((0, ph), (0, pw), (0, 0)), mode="edge")
    h, w, _ = x.shape
    x = x.reshape(h // f, f, w, c).sum(1, dtype=np.float32)
    return x.reshape(h // f, w // f, f, c).sum(2, dtype=np.float32) * np.float32(1.0 / (f * f))


def _upsample(x, f, h, w):
    """Separable bilinear upsample of (h',w',3) back to (h,w,3), pixel-centre aligned."""
    for axis, n in ((0, h), (1, w)):
        src = np.clip((np.arange(n, dtype=np.float32) + 0.5) / f - 0.5, 0, x.shape[axis] - 1)
        i0 = np.floor(src).astype(np.intp)
        i1 = np.minimum(i0 + 1, x.shape[axis] - 1)
        t = (src - i0).astype(np.float32)
        t = t.reshape((n, 1, 1) if axis == 0 else (1, n, 1))
        a0, a1 = np.take(x, i0, axis=axis), np.take(x, i1, axis=axis)
        a1 -= a0
        a1 *= t
        a0 += a1
        x = a0
    return x


def blur(img, sigma, fast=False):
    """Approximate Gaussian blur of an (H,W,3) float32 image.

    fast=True: radii >= 4 px are computed at 1/f resolution (f = power of two <= sigma/2, max 8) and
    bilinearly upsampled (mean error ~0.2%, up to ~3% on hard edges, up to ~2x faster).
    Default is the exact full-resolution path (it costs about the same at every radius).
    """
    if sigma < 0.4:
        return img
    h, w, _ = img.shape
    x = np.ascontiguousarray(img, dtype=np.float32)
    f = 1
    while fast and f < 8 and sigma / (f * 2) >= 2.0 and min(h, w) // (f * 2) >= 8:
        f *= 2
    if f > 1:
        return _upsample(_gauss3(_reduce(x, f), sigma / f), f, h, w)
    return _gauss3(x, sigma)


def remap_colors(img, rules, threshold_offset=0.0, amount_scale=1.0, hue_shift=0.0):
    """Assign source colours to target colours with a distance threshold.

    Distance is OKLab euclidean (~0 identical, ~1 black-vs-white). A pixel
    within `threshold` of a rule's `from` colour is fully replaced; the mask
    fades to zero over `softness` beyond that. Masks are all computed against
    the *input* so rules never chain into each other.
    """
    if not rules:
        return img
    h, w, _ = img.shape
    lab = rgb_to_oklab_fast(img.reshape(-1, 3))
    lab2 = np.einsum("ij,ij->i", lab, lab)          # |lab|^2, shared by every rule
    out = img.reshape(-1, 3).copy()
    for r in rules:
        amt = float(np.clip(r.get("amount", 1.0) * amount_scale, 0, 1))
        if amt <= 0:
            continue
        src = rgb_to_oklab(parse_color(r["from"])).astype(np.float32)
        tgt = oklab_to_rgb(rotate_hue_lab(rgb_to_oklab(parse_color(r["to"])), hue_shift)).astype(np.float32)
        thr = max(0.0, r.get("threshold", 0.2) + threshold_offset)
        soft = max(1e-4, r.get("softness", 0.05))
        d2 = lab @ (-2.0 * src)                      # |lab - src|^2 = |lab|^2 - 2 lab.src + |src|^2
        d2 += lab2
        d2 += float(src @ src)
        np.maximum(d2, 0.0, out=d2)
        if d2.min() >= (thr + soft) ** 2:            # nothing in range: skip the blend entirely
            continue
        t = np.sqrt(d2, out=d2)
        t -= thr
        t *= np.float32(1.0 / soft)
        np.clip(t, 0.0, 1.0, out=t)
        m = t * t                                    # mask = amt * (1 - smoothstep(t))
        t *= -2.0
        t += 3.0
        m *= t
        np.subtract(1.0, m, out=m)
        m *= np.float32(amt)
        out += m[:, None] * (tgt - out)              # lerp toward target by mask
    return out.reshape(h, w, 3)
