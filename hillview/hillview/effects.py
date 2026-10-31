"""Stateless image operators. All take/return float32 RGB in [0,1], shape (H,W,3)."""
import numpy as np
from .color import (rgb_to_oklab, oklab_to_rgb, parse_color, rotate_hue_lab)


def posterize(img, levels):
    levels = max(2, int(round(levels)))
    return np.round(img * (levels - 1)) / (levels - 1)


def kmeans_quantize(img, k, seed=0, iters=8):
    """Palette quantize to k colours. Deterministic for a given seed."""
    k = max(2, int(round(k)))
    h, w, _ = img.shape
    flat = img.reshape(-1, 3)
    rng = np.random.default_rng(seed)
    sub = flat[rng.integers(0, len(flat), size=min(4096, len(flat)))]
    cent = [sub[rng.integers(len(sub))]]
    for _ in range(k - 1):  # k-means++ init
        d = np.min([((sub - c) ** 2).sum(1) for c in cent], axis=0)
        s = d.sum()
        cent.append(sub[rng.choice(len(sub), p=d / s) if s > 0 else rng.integers(len(sub))])
    cent = np.array(cent, np.float32)
    for _ in range(iters):
        idx = ((sub[:, None] - cent[None]) ** 2).sum(2).argmin(1)
        for j in range(k):
            m = idx == j
            if m.any():
                cent[j] = sub[m].mean(0)
    out = np.empty_like(flat)
    for s in range(0, len(flat), 262144):  # chunked to bound memory
        c = flat[s:s + 262144]
        out[s:s + 262144] = cent[((c[:, None] - cent[None]) ** 2).sum(2).argmin(1)]
    return out.reshape(h, w, 3)


def _box(a, r, axis):
    """Box blur of radius r along axis via cumulative sum (O(1) in r)."""
    if r <= 0:
        return a
    pad = [(0, 0)] * a.ndim
    pad[axis] = (r + 1, r)
    c = np.cumsum(np.pad(a, pad, mode="edge"), axis=axis, dtype=np.float64)
    n = a.shape[axis]
    hi = np.take(c, range(2 * r + 1, 2 * r + 1 + n), axis=axis)
    lo = np.take(c, range(0, n), axis=axis)
    return ((hi - lo) / (2 * r + 1)).astype(np.float32)


def blur(img, sigma):
    """Approximate Gaussian: three box passes per axis (error < ~3%)."""
    if sigma < 0.4:
        return img
    r = max(1, int(round((np.sqrt(4 * sigma * sigma + 1) - 1) / 2)))  # 3 boxes: var = 3*(w^2-1)/12
    out = img
    for _ in range(3):
        out = _box(_box(out, r, 0), r, 1)
    return out


def remap_colors(img, rules, threshold_offset=0.0, amount_scale=1.0, hue_shift=0.0):
    """Assign source colours to target colours with a distance threshold.

    Distance is OKLab euclidean (~0 identical, ~1 black-vs-white). A pixel
    within `threshold` of a rule's `from` colour is fully replaced; the mask
    fades to zero over `softness` beyond that. Masks are all computed against
    the *input* so rules never chain into each other.
    """
    if not rules:
        return img
    lab = rgb_to_oklab(img)
    out = img.copy()
    for r in rules:
        src = rgb_to_oklab(parse_color(r["from"]))
        tgt = rotate_hue_lab(rgb_to_oklab(parse_color(r["to"])), hue_shift)
        d = np.sqrt(((lab - src) ** 2).sum(-1))
        thr = max(0.0, r.get("threshold", 0.2) + threshold_offset)
        soft = max(1e-4, r.get("softness", 0.05))
        t = np.clip((d - thr) / soft, 0, 1)
        mask = 1 - t * t * (3 - 2 * t)
        mask = mask * np.clip(r.get("amount", 1.0) * amount_scale, 0, 1)
        out = out * (1 - mask[..., None]) + oklab_to_rgb(tgt)[None, None, :] * mask[..., None]
    return out
