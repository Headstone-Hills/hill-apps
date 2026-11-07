"""Sequencer engine: tick-based, with its own clock thread.

Design (see README "Timing"):
  * 24 ticks per quarter note; a step is `groove.ticks[step]` ticks (6 = a 16th).
  * `_compute_tick()` is pure bookkeeping: it advances every track, runs the
    scheduled-event queue for this tick and returns ONE bytes object of USB-MIDI
    packets. It never sleeps, draws or touches files, so tests drive it directly.
  * The clock thread computes tick N+1 right after sending tick N, sleeps to just
    before the absolute deadline (time.monotonic_ns), spins the last ~1.5 ms, and
    sends the whole tick in a single bulk write.
  * Events are sent `latency_ms` early to compensate for FM-1 latency.
"""

from __future__ import annotations

import threading
import time

import fm1_usb as usb
import fx as fxmod
from model import (EMPTY, NOTE_OFF, NUM_TRACKS, PHRASE_LEN, clamp)

PPQ = 24
SPIN_NS = 1_500_000
RELEASE = -1            # queue sentinel: leave a live-queued chain, rejoin the song


class JitterStats(object):
    """Scheduling error = (actual send time - intended send time), in microseconds."""
    def __init__(self):
        self.reset()

    def reset(self):
        self.n = 0
        self.total = 0
        self.min = None
        self.max = 0
        self.late = 0              # > 1 ms
        self.recent = [0] * 128
        self.write_us_max = 0

    def add(self, err_ns):
        us = err_ns // 1000
        self.n += 1
        self.total += us
        self.min = us if self.min is None or us < self.min else self.min
        self.max = us if us > self.max else self.max
        if us > 1000:
            self.late += 1
        self.recent[self.n % 128] = us

    @property
    def avg(self):
        return self.total / self.n if self.n else 0.0


class TrackState(object):
    def __init__(self, idx):
        self.idx = idx
        self.mute = False
        self.solo = False
        self.activity = (0, 0)          # (monotonic_ns, velocity) for the mixer meter
        self.reset()

    def reset(self):
        self.active = False
        self.started = False
        self.countdown = 0
        self.row = 0
        self.chain_idx = EMPTY
        self.chain_step = 0
        self.phrase_idx = EMPTY
        self.phrase_step = 0
        self.xpose = 0
        self.instr = 0
        self.groove = 0
        self.tps = 0
        self.vol = 128
        self.hop = None
        self.hop_counts = {}
        self.queued = None
        self.live = False
        self.events = []
        self.sounding = set()
        self.gen = 0
        self.tbl_pos = 0
        self.tbl_note = None


class Engine(object):
    def __init__(self, transport, project, clock=time.monotonic_ns):
        self.transport = transport
        self.p = project
        self.clock = clock
        self.lock = threading.RLock()
        self._send_lock = threading.Lock()
        self._session = 0
        self.tracks = [TrackState(i) for i in range(NUM_TRACKS)]
        self.stats = JitterStats()
        self.playing = False
        self.mode = "song"              # song | chain | phrase
        self.bpm = project.bpm
        self.tick = 0
        self._seq = 0
        self._out = bytearray()
        self._count = {}                # (ch, note) -> refcount across tracks
        self._chan_instr = {}           # channel -> instrument index last selected
        self._bent = set()
        self._used_ch = set([project.midi_channel])
        self._timed = []                # (due_ns, ch, note) for previews
        self._pending = None
        self._next_ns = 0
        self._finish = False
        self._quit = False
        self._wake = threading.Event()
        self._thread = None
        self.on_stop = None             # callback(engine) when playback ends by itself

    # ------------------------------------------------------------------ thread
    def start_thread(self):
        if self._thread is None:
            self._thread = threading.Thread(target=self._run, name="fm1-clock", daemon=True)
            self._thread.start()

    def shutdown(self):
        """Stop, release every note, end the thread. Safe to call repeatedly."""
        self._quit = True
        self._wake.set()
        try:
            self.stop()
            self.panic()
        except Exception:
            pass
        t = self._thread
        if t is not None and t is not threading.current_thread():
            t.join(0.5)

    def _tick_ns(self):
        return int(60_000_000_000 / (max(20, self.bpm) * PPQ))

    def _run(self):
        while not self._quit:
            now = self.clock()
            self._service_timed(now)
            if not self.playing:
                self._wake.wait(0.004)
                self._wake.clear()
                continue
            if self._pending is None:
                with self.lock:
                    if not self.playing:
                        continue
                    session = self._session
                    self._pending = (session, self._compute_tick())
                    self._next_ns_use = self._next_ns
                    self._next_ns += self._tick_ns()
            session, data = self._pending
            send_at = self._next_ns_use - self.p.latency_ms * 1_000_000
            while self.playing and not self._quit:
                now = self.clock()
                rem = send_at - now
                if rem <= 0:
                    break
                if rem > SPIN_NS + 500_000:
                    time.sleep(min(0.02, (rem - SPIN_NS) / 1e9))
                # else: spin the last ~1.5 ms
            self._pending = None
            if not self.playing or self._quit:
                continue
            with self._send_lock:
                if session == self._session:
                    err = self.clock() - send_at
                    self._send(data)
                    self.stats.add(err if err > 0 else 0)
            if self._finish:
                self._finish = False
                self.stop(natural=True)

    def _send(self, data):
        if data:
            self.transport.send(data)
            self.stats.write_us_max = max(self.stats.write_us_max, getattr(self.transport, "last_write_us", 0))

    def _flush_now(self):
        with self.lock:
            data, self._out = bytes(self._out), bytearray()
        self._send(data)

    def _service_timed(self, now):
        if not self._timed:
            return
        with self.lock:
            due = [t for t in self._timed if t[0] <= now]
            if not due:
                return
            self._timed = [t for t in self._timed if t[0] > now]
            for _, ch, note in due:
                self._out += usb.pkt_note_off(ch, note)
        self._flush_now()

    # ------------------------------------------------------------ transport ops
    def _begin(self, mode):
        self.stop(silent=True)
        with self.lock:
            self.mode = mode
            self.bpm = self.p.bpm
            self.tick = 0
            for tr in self.tracks:
                tr.reset()
                tr.groove = self.p.groove
            self._finish = False
            self._pending = None

    def _go(self):
        with self.lock:
            self._next_ns = self.clock() + self.p.latency_ms * 1_000_000 + 3_000_000
            self._session += 1
            self.playing = True
        self._wake.set()

    def play_song(self, row=0):
        """Play all tracks from song row `row`. Returns False if the song is empty."""
        if self.p.song_last_row() < 0:
            return False
        self._begin("song")
        with self.lock:
            for tr in self.tracks:
                self._song_enter_row(tr, clamp(row, 0, len(self.p.song) - 1))
        self._go()
        return True

    def play_chain(self, track, chain_idx):
        self._begin("chain")
        with self.lock:
            tr = self.tracks[track]
            tr.active, tr.chain_idx, tr.chain_step = True, chain_idx, 0
            self._enter_phrase(tr)
        self._go()
        return True

    def play_phrase(self, track, phrase_idx):
        self._begin("phrase")
        with self.lock:
            tr = self.tracks[track]
            tr.active, tr.chain_idx, tr.phrase_idx = True, EMPTY, phrase_idx
        self._go()
        return True

    def stop(self, silent=False, natural=False):
        """Stop playback and release every sounding note. Idempotent."""
        with self._send_lock:
            self._session += 1
            was = self.playing
            self.playing = False
        with self.lock:
            self._pending = None
            self._release_everything()
        self._flush_now()
        if natural and was and self.on_stop:
            try:
                self.on_stop(self)
            except Exception:
                pass

    def panic(self):
        """Hard stop: CC123 + CC120 on all 16 channels, bend centre, clear state."""
        with self._send_lock:
            self._session += 1
            self.playing = False
        with self.lock:
            self._pending = None
            for tr in self.tracks:
                tr.events = []
                tr.sounding.clear()
            self._count.clear()
            self._timed = []
            self._out += usb.panic_packets(range(16))
            for ch in range(16):
                self._out += usb.pkt_pitchbend(ch, 8192)
            self._bent.clear()
        self._flush_now()

    def _release_everything(self):
        for tr in self.tracks:
            tr.events = []
            self._release_track(tr)
        self._count.clear()
        for ch in sorted(self._used_ch):
            self._out += usb.pkt_cc(ch, 123, 0)
        for ch in sorted(self._bent):
            self._out += usb.pkt_pitchbend(ch, 8192)
        self._bent.clear()
        self._timed = []

    def preview(self, ins_idx, note, vel=None, ms=300):
        """Audition a note immediately (works while stopped or playing)."""
        with self.lock:
            ins = self.p.instrument(ins_idx)
            self._used_ch.add(ins.channel)
            if self._chan_instr.get(ins.channel) != ins_idx:
                self._select(ins.channel, ins_idx)
            v = clamp(ins.velocity if vel is None else vel, 1, 127)
            self._out += usb.pkt_note_on(ins.channel, clamp(note, 0, 127), v)
            self._timed.append((self.clock() + ms * 1_000_000, ins.channel, clamp(note, 0, 127)))
        self._flush_now()

    def invalidate_instrument(self, ins_idx=None):
        """Force program/CC re-send on the next note (call after editing instruments)."""
        with self.lock:
            for ch, i in list(self._chan_instr.items()):
                if ins_idx is None or i == ins_idx:
                    self._chan_instr[ch] = None

    # ---------------------------------------------------------- mute / solo / queue
    def audible(self, tr):
        if any(t.solo for t in self.tracks):
            return tr.solo
        return not tr.mute

    def set_mute(self, track, value):
        with self.lock:
            tr = self.tracks[track]
            tr.mute = value
            if not self.audible(tr):
                self._release_track(tr)
        self._flush_now()

    def toggle_solo(self, track):
        with self.lock:
            tr = self.tracks[track]
            tr.solo = not tr.solo
            for t in self.tracks:
                if not self.audible(t):
                    self._release_track(t)
        self._flush_now()

    def queue_chain(self, track, chain_idx):
        """Live-queue: at the end of the track's current chain switch to `chain_idx`
        and loop it. chain_idx=None releases back to the song order."""
        with self.lock:
            self.tracks[track].queued = RELEASE if chain_idx is None else chain_idx

    # -------------------------------------------------------------- note plumbing
    def _release_track(self, tr):
        for ch, note in list(tr.sounding):
            self._off_counted(ch, note)
        tr.sounding.clear()

    def _off_counted(self, ch, note):
        n = self._count.get((ch, note), 0) - 1
        if n <= 0:
            self._count.pop((ch, note), None)
            self._out += usb.pkt_note_off(ch, note)
        else:
            self._count[(ch, note)] = n

    def _select(self, ch, ins_idx):
        """Send bank/program/CC mappings for an instrument (releases the channel first)."""
        for tr in self.tracks:
            for c, n in [x for x in tr.sounding if x[0] == ch]:
                tr.sounding.discard((c, n))
                self._off_counted(c, n)
        ins = self.p.instrument(ins_idx)
        if ins.bank_msb != EMPTY:
            self._out += usb.pkt_cc(ch, 0, ins.bank_msb)
        if ins.bank_lsb != EMPTY:
            self._out += usb.pkt_cc(ch, 32, ins.bank_lsb)
        self._out += usb.pkt_pc(ch, ins.program)
        self._send_ccs(ch, ins)
        self._chan_instr[ch] = ins_idx

    def _send_ccs(self, ch, ins):
        for cc, val in ins.ccs:
            if cc != EMPTY:
                self._out += usb.pkt_cc(ch, cc, val)

    def _at(self, tr, due, kind, *args):
        prio = 0 if kind in ("off", "reloff") else 2 if kind == "on" else 1
        self._seq += 1
        tr.events.append((due, prio, self._seq, kind, args))

    # ------------------------------------------------------------------ the tick
    def process_tick(self):
        """Compute one tick and return its packet bytes (tests / headless use)."""
        with self.lock:
            return self._compute_tick()

    def _step_track(self, tr, tick):
        if tr.active:
            if tr.countdown <= 0:
                if tr.started:
                    self._advance(tr)
                tr.started = True
                if tr.active:
                    tr.countdown = self._trigger(tr, tick)
            tr.countdown -= 1
        if tr.events:
            self._run_events(tr, tick)

    def _compute_tick(self):
        tick = self.tick
        for tr in self.tracks:
            self._step_track(tr, tick)
        if self.mode == "song" and not any(t.active for t in self.tracks):
            if self.p.loop_song:
                # restart in the SAME tick so the loop point has no gap
                for tr in self.tracks:
                    tr.started, tr.countdown = False, 0
                    self._song_enter_row(tr, 0)
                    self._step_track(tr, tick)
            elif not any(t.events for t in self.tracks):
                self._finish = True
        elif self.mode != "song" and not any(t.active or t.events for t in self.tracks):
            self._finish = True
        self.tick = tick + 1
        data, self._out = bytes(self._out), bytearray()
        return data

    def _run_events(self, tr, tick):
        due = [e for e in tr.events if e[0] <= tick]
        if not due:
            return
        tr.events = [e for e in tr.events if e[0] > tick]
        due.sort(key=lambda e: (e[0], e[1], e[2]))
        for _, _, _, kind, a in due:
            if kind == "off":
                ch, note = a
                if (ch, note) in tr.sounding:
                    tr.sounding.discard((ch, note))
                    self._off_counted(ch, note)
            elif kind == "reloff":
                self._release_track(tr)
            elif kind == "on":
                self._ev_on(tr, *a)
            elif kind == "cc":
                if self.audible(tr):
                    self._out += usb.pkt_cc(a[0], a[1], a[2])
            elif kind == "pc":
                if self.audible(tr):
                    if self._chan_instr.get(a[0]) != a[2]:
                        self._select(a[0], a[2])       # PCH overrides the instrument's program,
                    self._out += usb.pkt_pc(a[0], a[1])   # but only after the instrument is set up
            elif kind == "pb":
                if self.audible(tr):
                    self._out += usb.pkt_pitchbend(a[0], a[1])
                    self._bent.add(a[0])
            elif kind == "tbl":
                self._ev_table(tr, tick, *a)

    def _ev_on(self, tr, ch, note, vel, ins_idx):
        if not self.audible(tr):
            return
        self._release_track(tr)                                  # tracks are monophonic
        self._used_ch.add(ch)
        ins = self.p.instrument(ins_idx)
        if self._chan_instr.get(ch) != ins_idx:
            self._select(ch, ins_idx)
        elif ins.send_mode == 1:
            self._send_ccs(ch, ins)
        note = clamp(note, 0, 127)
        self._out += usb.pkt_note_on(ch, note, vel)
        tr.sounding.add((ch, note))
        self._count[(ch, note)] = self._count.get((ch, note), 0) + 1
        tr.activity = (self.clock(), vel)

    def _ev_table(self, tr, tick, gen, tbl_idx, row_i, ins_idx, base, vel, ch):
        if tr.gen != gen:
            return
        tb = self.p.tables.get(tbl_idx)
        if tb is None:
            return
        row = tb.rows[row_i]
        note = clamp(base + row.trn, 0, 127)
        if row_i > 0 and tr.tbl_note is not None and note != tr.tbl_note:
            self._ev_on(tr, ch, note, clamp(vel + row.vel, 1, 127), ins_idx)
        tr.tbl_note = note
        ctx = fxmod.StepCtx()
        for cmd, val in row.fx:
            if cmd:
                fxmod.run(cmd, ctx, val)
        ins = self.p.instrument(ins_idx)
        if ctx.kill is not None:
            self._at(tr, tick + ctx.kill, "off", ch, note)
        if ctx.bend is not None:
            self._at(tr, tick, "pb", ch, ctx.bend)
        for v in ctx.ccs:
            self._at(tr, tick, "cc", ch, ins.mcc_cc, v)
        if ctx.vol is not None:
            tr.vol = ctx.vol

    # ------------------------------------------------------------- song position
    def _chain_len(self, tr):
        c = self.p.chains.get(tr.chain_idx) if tr.chain_idx != EMPTY else None
        return c.length() if c else 1

    def _song_enter_row(self, tr, row):
        last = self.p.song_last_row()
        tr.row = row
        if row > last:
            tr.active = False
            return
        tr.active = True
        tr.chain_idx = self.p.song[row][tr.idx]
        tr.chain_step = 0
        self._enter_phrase(tr)

    def _enter_phrase(self, tr):
        c = self.p.chains.get(tr.chain_idx) if tr.chain_idx != EMPTY else None
        if c is not None:
            tr.phrase_idx = c.phrase[tr.chain_step]
            tr.xpose = c.transpose[tr.chain_step]
        else:
            tr.phrase_idx, tr.xpose = EMPTY, 0
        tr.phrase_step = 0
        tr.hop_counts = {}

    def _advance(self, tr):
        if tr.hop is not None:                           # HOP
            v, tr.hop = tr.hop, None
            reps, target = v >> 4, v & 15
            if reps != 0:
                key = (tr.phrase_idx, tr.phrase_step)
                n = tr.hop_counts.get(key, 0)
                if reps == 15 or n < reps:
                    tr.hop_counts[key] = n + 1
                    tr.phrase_step = target
                    return
                tr.hop_counts[key] = 0
            else:
                return self._end_phrase(tr)
        tr.phrase_step += 1
        if tr.phrase_step >= PHRASE_LEN:
            self._end_phrase(tr)

    def _end_phrase(self, tr):
        if self.mode == "phrase":
            tr.phrase_step = 0
            tr.hop_counts = {}
            return
        tr.chain_step += 1
        if tr.chain_step < self._chain_len(tr):
            self._enter_phrase(tr)
            return
        # end of chain
        q, tr.queued = tr.queued, None
        if q is not None and q != RELEASE:
            tr.chain_idx, tr.chain_step, tr.live = q, 0, True
            self._enter_phrase(tr)
        elif self.mode == "chain" or (tr.live and q != RELEASE):
            tr.chain_step = 0
            self._enter_phrase(tr)
        else:
            tr.live = False
            self._song_enter_row(tr, tr.row + 1)

    def _step_len(self, tr):
        if tr.tps:
            return tr.tps
        g = self.p.grooves.get(tr.groove) or self.p.grooves[0]
        return max(1, g.ticks[tr.phrase_step % PHRASE_LEN])

    # ------------------------------------------------------------- step trigger
    def _trigger(self, tr, tick):
        """Run the step under `tr`'s cursor. Returns the step length in ticks."""
        ph = self.p.phrases.get(tr.phrase_idx) if tr.phrase_idx != EMPTY else None
        if ph is None:
            return self._step_len(tr)
        st = ph.steps[tr.phrase_step]
        ctx = fxmod.StepCtx()
        for cmd, val in st.fx:
            if cmd:
                fxmod.run(cmd, ctx, val)
        if st.instr != EMPTY:
            tr.instr = st.instr
        if ctx.groove is not None:
            tr.groove = ctx.groove
        if ctx.tps is not None:
            tr.tps = ctx.tps
        if ctx.vol is not None:
            tr.vol = ctx.vol
        if ctx.tempo:
            self.bpm = clamp(ctx.tempo, 20, 300)
        tr.hop = ctx.hop
        step_len = self._step_len(tr)
        ins = self.p.instrument(tr.instr)
        ch = ins.channel
        t0 = tick + clamp(ctx.delay, 0, step_len - 1)
        if ctx.pc is not None:
            self._at(tr, t0, "pc", ch, ctx.pc, tr.instr)
        if ctx.bend is not None:
            self._at(tr, t0, "pb", ch, ctx.bend)
        for v in ctx.ccs:
            self._at(tr, t0, "cc", ch, ins.mcc_cc, v)
        if st.note == NOTE_OFF:
            self._at(tr, t0, "reloff")
        elif st.note != EMPTY:
            self._schedule_note(tr, st, ins, ctx, tick, t0, step_len)
        return step_len

    def _schedule_note(self, tr, st, ins, ctx, tick, t0, step_len):
        ch = ins.channel
        base = st.note + tr.xpose + self.p.transpose + ins.transpose
        vel = st.vel if st.vel != EMPTY else ins.velocity
        vel = vel * tr.vol // 128
        tb = self.p.tables.get(ins.table) if ins.table != EMPTY else None
        tr.gen += 1
        if tb is not None and tb.mode == 0:                 # per-note table
            L = tb.length()
            row = tb.rows[tr.tbl_pos % L]
            tr.tbl_pos += 1
            base, vel = base + row.trn, vel + row.vel
            for cmd, val in row.fx:
                if cmd:
                    fxmod.run(cmd, ctx, val)
        elif tb is not None:                                # tick-mode table
            tr.tbl_note = clamp(base + tb.rows[0].trn, 0, 127)
            base += tb.rows[0].trn
            for r in range(tb.length()):
                self._at(tr, t0 + r * max(1, ins.table_rate), "tbl",
                         tr.gen, ins.table, r, tr.instr,
                         st.note + tr.xpose + self.p.transpose + ins.transpose,
                         clamp(vel, 1, 127), ch)
        vel = clamp(vel, 1, 127)
        end = t0 + (ctx.kill if ctx.kill is not None else (ins.gate or (step_len - (t0 - tick))))
        end = max(end, t0 + 1)
        ins_i = tr.instr
        if ctx.arp is not None:
            offs = (0, ctx.arp[0], ctx.arp[1])
            rate = max(1, self.p.arp_rate)
            i, k = t0, 0
            while i < end:
                self._at(tr, i, "on", ch, base + offs[k % 3], vel, ins_i)
                i, k = i + rate, k + 1
            self._at_off_all(tr, end, ch, [base + o for o in offs])
        elif ctx.retrig is not None:
            interval, decay = ctx.retrig
            i, k = t0, 0
            while i < end:
                self._at(tr, i, "on", ch, base, clamp(vel - k * decay, 1, 127), ins_i)
                i, k = i + interval, k + 1
            self._at(tr, end, "off", ch, clamp(base, 0, 127))
        else:
            self._at(tr, t0, "on", ch, base, vel, ins_i)
            self._at(tr, end, "off", ch, clamp(base, 0, 127))

    def _at_off_all(self, tr, due, ch, notes):
        for n in notes:
            self._at(tr, due, "off", ch, clamp(n, 0, 127))
