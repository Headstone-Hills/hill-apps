"""HillView reference engine: abstract / avant-garde video processing.

Pure-function-of-time design: every frame is computed from (source frames,
config, seed, time) with no hidden state, so the same algorithm can be ported
to hosts that render frames out of order (OpenFX, AE/Premiere, FxPlug, AVX2).
"""
__version__ = "0.1.0"
