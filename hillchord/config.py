"""HillChord global configuration constants.

All environment-specific paths and tunables live here so swapping between
the Mac dev box and the RG35XXSP on muOS is a one-line change.
"""

import os
import sys

# --- Platform detection -----------------------------------------------------
# On the device the SD card is mounted at /mnt/sdcard. Locally it is not.
ON_DEVICE = os.path.isdir("/mnt/sdcard/ROMs")

# --- Paths ------------------------------------------------------------------
if ON_DEVICE:
    SAMPLE_PATH = "/mnt/sdcard/ROMs/Samples"
    STATE_PATH = "/mnt/sdcard/ROMs/Samples/hillchord_state.json"
    # Persistent render cache (kept OUTSIDE Samples so it isn't browsed).
    RENDER_CACHE_DIR = "/mnt/sdcard/ROMs/.hillchord_cache"
else:
    _HERE = os.path.dirname(os.path.abspath(__file__))
    SAMPLE_PATH = os.path.join(_HERE, "samples")
    STATE_PATH = os.path.join(_HERE, "hillchord_state.json")
    RENDER_CACHE_DIR = os.path.join(_HERE, ".render_cache")

# --- Display ----------------------------------------------------------------
SCREEN_W = 640
SCREEN_H = 480
FPS = 60

# --- Mixer ------------------------------------------------------------------
MIXER_FREQ = 44100
MIXER_SIZE = -16          # signed 16-bit
MIXER_CHANNELS_OUT = 2    # stereo output
MIXER_BUFFER = 512        # small buffer for low latency
NUM_VOICES = 12           # simultaneous sounds (spec requirement)

# --- Audio / DSP ------------------------------------------------------------
SAMPLE_RATE = MIXER_FREQ

# --- Timing -----------------------------------------------------------------
DOUBLE_TAP_MS = 300       # double-tap window (spec)
LOOP_CANCEL_HOLD_MS = 2000  # hold L2 2s to cancel loop (spec)
EXIT_HOLD_SECONDS = 1.2   # Select+Start hold -> quit

# --- Looper -----------------------------------------------------------------
# Timing forgiveness at the loop boundary: notes played up to this far before the
# downbeat snap to the start, and notes this close to the loop end snap to the
# start too. Mid-phrase timing is preserved; loop length stays beat-locked.
LOOP_EDGE_MS = 120

# --- BPM (spec: library left/right adjusts) --------------------------------
BPM_MIN = 40
BPM_MAX = 240
BPM_STEP = 5
BPM_DEFAULT = 120

# --- Wet/Dry steps (spec: R2 cycles 0/25/50/75/100/0) ----------------------
WETDRY_STEPS = [0, 25, 50, 75, 100]

# --- Bitcrush (lo-fi output effect, baked -> no runtime overhead) -----------
# Adjustable in-app: R1+R2 cycles bit depth, L1+R2 cycles downsampling.
CRUSH_BITS_STEPS = [16, 12, 8, 4]      # 16 = clean; stages of 4
CRUSH_DOWN_STEPS = [1, 2, 4, 6, 8]     # 1 = off; stages of 2 (sample-and-hold)

# --- Octave range (Function + D-Up/Down) -----------------------------------
OCTAVE_MIN = -3
OCTAVE_MAX = 3

# --- Arpeggiator ------------------------------------------------------------
ARP_DIVISION = 2          # steps per beat (2 = eighth notes)
ARP_GATE_MS = 90          # fade-out of each arp note when the next fires, so
                          # notes stay distinct instead of sustaining/piling up

# --- Render cache -----------------------------------------------------------
# Rendered (pitch-shifted + effected) notes are cached to disk (compute once,
# ever) and a small in-RAM LRU. Trades storage for CPU/RAM — ideal for a device
# with lots of storage but little memory.
RAM_CACHE_BYTES = 64 * 1024 * 1024   # ~64 MB of decoded audio kept in RAM

# --- Pitch / tempo ----------------------------------------------------------
# For single-sample LOOPS (pads/drones/rhythmic loops), keep the original
# tempo when pitch-shifting (time-stretch) instead of repitching like vinyl.
# Set False for classic resample (pitch changes speed too).
PITCH_PRESERVE_TEMPO = True

# Time-stretch looping voices so they ride the metronome tempo (pitch
# unchanged). Native tempo is read from the filename when present (e.g.
# "120_A_IcePad", "90bpm"); otherwise the loop length is snapped to the nearest
# whole number of beats. The snap is skipped when it would need an extreme
# stretch, so long free-time drones aren't mangled.
LOOP_MATCH_TEMPO = True
LOOP_SNAP_MIN_RATIO = 0.66   # don't snap-stretch beyond these bounds (drones)
LOOP_SNAP_MAX_RATIO = 1.5

# --- Loop recorder quantize -------------------------------------------------
# Notes captured into the looper snap to the nearest 1/Nth of a beat. 4 = a
# quarter-beat (16th note). Set 0 to disable.
LOOP_QUANTIZE_DIV = 4

# --- Level normalization (per loaded sound) ---------------------------------
# Cheap RMS-match so wildly different packs (quiet VCSL vs loud chip samples)
# play at a similar loudness. Gain is peak-limited to avoid clipping.
NORMALIZE = True
NORM_TARGET_RMS = 0.12    # fraction of full scale (~ -18 dBFS)
NORM_PEAK_LIMIT = 0.95    # never let normalization push the peak past this
NORM_MAX_GAIN = 8.0       # don't over-amplify near-silent samples

# --- Sustained-voice loop crossfade -----------------------------------------
# Blend each looping voice's tail into its head so held notes/drones loop
# seamlessly even when the source sample wasn't authored to loop.
LOOP_XFADE_MS = 60

# --- Defaults for a fresh state --------------------------------------------
DEFAULT_KEY = "C"
DEFAULT_MODE = "chord"    # "chord" | "note"

# --- Fonts ------------------------------------------------------------------
FONT_SIZE_SMALL  = 22
FONT_SIZE_NORMAL = 30
FONT_SIZE_LARGE  = 44
FONT_SIZE_HUGE   = 64
