"""HillBand — global configuration.

16-step × 8-track sequencer, split personality:
  * Tracks 0–3 (melodic): HillChord pitched sampler, chord/scale mode, effects
  * Tracks 4–7 (drums):   one-shot percussion, no pitch-shift, no effects
"""

import os

# --- Platform detection -------------------------------------------------------
ON_DEVICE = os.path.isdir("/mnt/sdcard/ROMs")

_APP_DIR = os.path.dirname(os.path.abspath(__file__))

# --- Paths -------------------------------------------------------------------
# Samples shared with HillChord/HillSequencer (never copied).
if ON_DEVICE:
    _DEFAULT_SAMPLES      = "/mnt/sdcard/ROMs/Samples"
    _DEFAULT_DRUM_SAMPLES = "/mnt/sdcard/ports/hillbeat/samples/Cassette Drums"
    _DEFAULT_CACHE        = "/mnt/sdcard/ROMs/.hillband_cache"
else:
    _DEFAULT_SAMPLES      = os.path.normpath(os.path.join(_APP_DIR, "..", "hillchord", "samples"))
    _DEFAULT_DRUM_SAMPLES = os.path.normpath(os.path.join(_APP_DIR, "..", "hillbeat", "samples", "Cassette Drums"))
    _DEFAULT_CACHE        = os.path.join(_APP_DIR, ".render_cache")

SAMPLE_PATH      = os.environ.get("HILLBAND_SAMPLES",      _DEFAULT_SAMPLES)
DRUM_SAMPLE_PATH = os.environ.get("HILLBAND_DRUM_SAMPLES", _DEFAULT_DRUM_SAMPLES)
RENDER_CACHE_DIR = os.environ.get("HILLBAND_CACHE",     _DEFAULT_CACHE)
STATE_PATH       = os.environ.get("HILLBAND_STATE",     os.path.join(_APP_DIR, "hillband_state.json"))
SEQUENCE_DIR     = os.environ.get("HILLBAND_SEQUENCES", os.path.join(_APP_DIR, "sequences"))
LOOP_CACHE       = os.environ.get("HILLBAND_LOOPCACHE", "/tmp/hillband_loop_stretched.wav")

# --- Display -----------------------------------------------------------------
SCREEN_W = 640
SCREEN_H = 480
FPS = 60

# --- Layout ------------------------------------------------------------------
STATUS_BAR_H  = 30
LOOP_BAR_H    = 28
TRACK_LABEL_W = 122    # wider than HillSequencer to fit chord/scale badges

# --- Track split -------------------------------------------------------------
NUM_TRACKS        = 8
NUM_MELODIC       = 4          # tracks 0–3 (top half)
NUM_DRUM          = 4          # tracks 4–7 (bottom half)
DRUM_TRACK_OFFSET = NUM_MELODIC

MELODIC_VOICE_NAMES = ["M1", "M2", "M3", "M4"]
DRUM_VOICE_NAMES    = ["KICK", "SNARE", "HAT", "PERC"]
DEFAULT_VOICE_NAMES = MELODIC_VOICE_NAMES + DRUM_VOICE_NAMES

# --- Steps / patterns --------------------------------------------------------
NUM_STEPS    = 16
NUM_PATTERNS = 8

# Per-track default MIDI note.  Drum entries are placeholders (snapping handles them).
DEFAULT_TRACK_NOTES = [60, 55, 48, 43, 60, 60, 60, 60]

# --- Transport ---------------------------------------------------------------
DEFAULT_BPM   = 120
BPM_MIN       = 40
BPM_MAX       = 240
BPM_STEP      = 1
BPM_HOLD_DELAY = 0.5
BPM_HOLD_STEP  = 5

DEFAULT_SWING = 0
SWING_MIN     = 0
SWING_MAX     = 100
SWING_STEP    = 5
SWING_FACTOR  = 0.33

# --- Velocity / volume -------------------------------------------------------
DEFAULT_VELOCITY    = 100
VELOCITY_MIN        = 0
VELOCITY_MAX        = 127
VELOCITY_STEP       = 10

DEFAULT_TRACK_VOLUME = 1.0
TRACK_VOLUME_STEP    = 0.1

# --- Note range (melodic tracks) ---------------------------------------------
NOTE_MIN = 24    # C1
NOTE_MAX = 96    # C7

# --- Hold thresholds / tap tempo ---------------------------------------------
A_HOLD_THRESHOLD      = 0.3
EXIT_HOLD_SECONDS     = 1.2
TAP_BUFFER_SIZE       = 6
TAP_MIN_TAPS          = 3
TAP_RESET_GAP         = 2.0
TAP_FEEDBACK_DURATION = 1.0

# --- Mixer -------------------------------------------------------------------
MIXER_FREQ         = 44100
MIXER_SIZE         = -16
MIXER_CHANNELS_OUT = 2
MIXER_BUFFER       = 512
SAMPLE_RATE        = MIXER_FREQ

# 4 channels per track: melodic tracks play up to 4-note chords simultaneously;
# drum tracks only use 1 but the uniform allocation keeps math simple.
CHANNELS_PER_TRACK = 4
NUM_VOICES         = CHANNELS_PER_TRACK
METRONOME_CHANNEL  = NUM_TRACKS * CHANNELS_PER_TRACK     # 32
LOOP_CHANNEL       = METRONOME_CHANNEL + 1               # 33
TOTAL_CHANNELS     = LOOP_CHANNEL + 1                    # 34

# --- Effects (melodic tracks only) -------------------------------------------
WETDRY_STEPS     = [0, 25, 50, 75, 100]
CRUSH_BITS_STEPS = [16, 12, 8, 4]
CRUSH_DOWN_STEPS = [1, 2, 4, 6, 8]

# --- Render cache ------------------------------------------------------------
RAM_CACHE_BYTES = 6 * 1024 * 1024

# --- Pitch / tempo / normalization -------------------------------------------
PITCH_PRESERVE_TEMPO = True
LOOP_MATCH_TEMPO     = True
LOOP_SNAP_MIN_RATIO  = 0.66
LOOP_SNAP_MAX_RATIO  = 1.5
LOOP_XFADE_MS        = 60

NORMALIZE       = True
NORM_TARGET_RMS = 0.12
NORM_PEAK_LIMIT = 0.95
NORM_MAX_GAIN   = 8.0

# --- Fonts -------------------------------------------------------------------
FONT_SIZE_SMALL  = 16
FONT_SIZE_NORMAL = 20
FONT_SIZE_LARGE  = 26

# --- Colours -----------------------------------------------------------------
CLR_BG            = ( 18,  18,  24)
CLR_STATUS_BG     = ( 28,  28,  38)
CLR_VOICE_BG      = ( 24,  24,  32)
CLR_VOICE_ACTIVE  = ( 32,  36,  52)
CLR_STEP_OFF      = ( 45,  45,  60)
CLR_STEP_ON       = ( 80, 180, 255)
CLR_STEP_ON_LOW   = ( 50, 120, 190)
CLR_STEP_MUTED    = ( 40,  40,  55)
CLR_STEP_MUTED_ON = ( 55,  55,  75)
CLR_CURSOR        = (255, 220,  60)
CLR_PLAYHEAD      = (255,  80,  80)
CLR_LOOP_BG       = ( 22,  30,  22)
CLR_LOOP_BAR_FG   = ( 80, 200, 100)
CLR_TEXT          = (220, 220, 230)
CLR_TEXT_DIM      = (120, 120, 140)
CLR_BADGE_PLAY    = ( 60, 200,  60)
CLR_BADGE_PAUSE   = (200, 200,  60)
CLR_BADGE_STOP    = (160,  60,  60)
CLR_BADGE_CHAIN   = (100, 160, 255)
CLR_BADGE_LOOP    = (100, 220, 140)
CLR_BADGE_MET     = (200, 170, 255)
CLR_WARNING       = (255, 160,  40)
CLR_OVERLAY_BG    = ( 10,  10,  18, 220)
CLR_OVERLAY_ITEM  = ( 36,  36,  52)
CLR_OVERLAY_SEL   = ( 60,  80, 140)
CLR_WHITE         = (255, 255, 255)
CLR_BLACK         = (  0,   0,   0)

# Per-track effect badge colours (melodic tracks)
CLR_FX_REVERB = (120, 205, 200)
CLR_FX_DELAY  = (235, 195, 110)
CLR_FX_CHORUS = (190, 175, 220)
CLR_FX_CRUSH  = (235, 130, 110)

# Track section accent colours (used for the divider bar between halves)
CLR_MELODIC_ACCENT = ( 80, 150, 255)
CLR_DRUM_ACCENT    = (255, 120,  80)
