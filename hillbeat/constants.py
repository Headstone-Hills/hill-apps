"""
HillBeat — constants.py
All magic numbers and configuration in one place.
"""

import os

# ── Display ───────────────────────────────────────────────────────────────────
SCREEN_W = 640
SCREEN_H = 480
FPS      = 60

# ── Layout ────────────────────────────────────────────────────────────────────
STATUS_BAR_H = 38
LOOP_BAR_H   = 38
VOICE_LABEL_W = 62          # width of the voice-name column on the left

# ── Sequencer ─────────────────────────────────────────────────────────────────
NUM_VOICES   = 4
NUM_STEPS    = 16
NUM_PATTERNS = 8

DEFAULT_BPM      = 120
BPM_MIN          = 40
BPM_MAX          = 200
DEFAULT_SWING    = 0
SWING_MIN        = 0
SWING_MAX        = 100

DEFAULT_VELOCITY = 100
VELOCITY_MIN     = 0
VELOCITY_MAX     = 127
VELOCITY_STEP    = 10       # per d-pad tick in hold-A mode

DEFAULT_VOICE_NAMES = ["Kick", "Snare", "Hat", "Perc"]

# BPM hold-repeat
BPM_HOLD_DELAY = 0.5        # seconds before auto-repeat
BPM_HOLD_RATE  = 5          # BPM units per second while held
BPM_HOLD_STEP  = 5          # jump size while held

# A-button hold threshold for velocity mode
A_HOLD_THRESHOLD = 0.3      # seconds
EXIT_HOLD_SECONDS = 1.2     # Start+Select hold -> exit to menu

# Tap-tempo
TAP_BUFFER_SIZE   = 6
TAP_MIN_TAPS      = 3
TAP_RESET_GAP     = 2.0     # seconds
TAP_FEEDBACK_DURATION = 1.0 # seconds

# ── Mixer ─────────────────────────────────────────────────────────────────────
SAMPLE_RATE  = 44100
BUFFER_SIZE  = 512
NUM_CHANNELS = 8            # voices 0-3, loop 4, spare 5-7
LOOP_CHANNEL = 4

# ── Paths ─────────────────────────────────────────────────────────────────────
# Samples and state live inside the app's own directory by default, so the app
# is portable across dev (Mac) and every device/OS combination (R36S/ArkOS,
# RG35XXSP/muOS) regardless of how files were copied onto the card. Each can be
# overridden with an env var if you want a shared sample folder elsewhere.
_APP_DIR = os.path.dirname(os.path.abspath(__file__))

SAMPLE_ROOT = os.environ.get("HILLBEAT_SAMPLES", os.path.join(_APP_DIR, "samples"))
STATE_FILE  = os.environ.get("HILLBEAT_STATE",   os.path.join(_APP_DIR, "hillbeat_state.json"))
LOOP_CACHE  = os.environ.get("HILLBEAT_CACHE",   "/tmp/hillbeat_loop_stretched.wav")

# ── Swing ─────────────────────────────────────────────────────────────────────
SWING_FACTOR = 0.33

# ── Colours ───────────────────────────────────────────────────────────────────
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
CLR_WARNING       = (255, 160,  40)
CLR_OVERLAY_BG    = ( 10,  10,  18, 220)
CLR_OVERLAY_ITEM  = ( 36,  36,  52)
CLR_OVERLAY_SEL   = ( 60,  80, 140)
CLR_WHITE         = (255, 255, 255)
CLR_BLACK         = (  0,   0,   0)

# ── Default sample filenames ──────────────────────────────────────────────────
DEFAULT_VOICE_SAMPLES = ["kick_01.wav", "snare_01.wav", "hat_closed.wav", "perc_rim.wav"]

# ── Grid layout ───────────────────────────────────────────────────────────────
GRID_PADDING = 8    # pixels left of steps area
STEP_GAP     = 2    # pixels between step squares

# ── Fonts ─────────────────────────────────────────────────────────────────────
FONT_SIZE_SMALL  = 17
FONT_SIZE_NORMAL = 21
FONT_SIZE_LARGE  = 30

# ── Gamepad button indices (RG35XXSP / muOS — verified on hardware) ───────────
# Matches hillchord's JOY_BUTTONS map exactly.
BTN_A        = 3
BTN_B        = 4
BTN_X        = 6
BTN_Y        = 5
BTN_L1       = 7
BTN_R1       = 8
BTN_L2       = 12
BTN_R2       = 13
BTN_SELECT   = 9
BTN_START    = 10
BTN_FUNCTION = 11

HAT_LEFT  = (-1,  0)
HAT_RIGHT = ( 1,  0)
HAT_UP    = ( 0,  1)
HAT_DOWN  = ( 0, -1)

# ── Per-voice volume ──────────────────────────────────────────────────────────
VOICE_VOLUME_MIN  = 0.0
VOICE_VOLUME_MAX  = 1.0
VOICE_VOLUME_STEP = 0.05
VOICE_VOLUME_DEFAULT = 1.0

# Y-button hold threshold (same pattern as A for velocity)
Y_HOLD_THRESHOLD = 0.3
