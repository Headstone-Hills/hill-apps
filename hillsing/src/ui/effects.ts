// The Effects section: a panel per effect (on/off switch + knobs), a preset menu and a reset button. It only edits
// the FxModel; the audio rack follows the model (see audio/bus.ts), so the panel works before audio has started.

import { EFFECTS, FX_PRESETS, formatParam, type EffectId, type FxModel } from '../audio/fxparams';
import { createKnob, type Knob } from './knob';

interface Row {
  id: EffectId;
  param: string;
  knob: Knob;
}

export class EffectsPanel {
  private readonly rows: Row[] = [];
  private readonly toggles = new Map<EffectId, HTMLInputElement>();
  private readonly cards = new Map<EffectId, HTMLElement>();
  private readonly preset: HTMLSelectElement;

  constructor(
    private readonly grid: HTMLElement,
    presetSelect: HTMLSelectElement,
    resetButton: HTMLButtonElement,
    private readonly model: FxModel,
  ) {
    this.preset = presetSelect;
    for (const e of EFFECTS) this.buildCard(e.id);
    for (const pr of FX_PRESETS) {
      const o = document.createElement('option');
      o.value = pr.id;
      o.textContent = pr.name;
      this.preset.append(o);
    }
    const placeholder = document.createElement('option');
    placeholder.value = '';
    placeholder.textContent = 'Presets…';
    this.preset.prepend(placeholder);
    this.preset.value = '';
    this.preset.addEventListener('change', () => {
      if (this.preset.value) model.applyPreset(this.preset.value);
      this.preset.value = '';
      this.preset.blur();
    });
    resetButton.addEventListener('click', () => model.applyPreset('clean'));
    resetButton.addEventListener('pointerup', () => resetButton.blur());
    model.onChange((id) => {
      if (id === null) this.refreshAll(); // a preset or reset replaced everything
    });
    this.refreshAll();
  }

  /** Called once audio is running: say so on the Bitcrush card if its worklet could not load. */
  setBitcrushAvailable(ok: boolean): void {
    const card = this.cards.get('bitcrush');
    if (!card) return;
    let note = card.querySelector('.fx-warn');
    if (ok) {
      note?.remove();
      return;
    }
    if (!note) {
      note = document.createElement('div');
      note.className = 'fx-warn hint error';
      note.textContent = 'Bitcrush needs audio worklets, which this browser blocked here. The other effects still work.';
      card.querySelector('.fx-head')?.after(note);
    }
  }

  private buildCard(id: EffectId): void {
    const spec = EFFECTS.find((e) => e.id === id)!;
    const card = document.createElement('div');
    card.className = 'fx-card';
    card.dataset.fx = id;

    // The checkbox stays as the stored on/off state (and for scripts); the footswitch and LED are its face.
    const on = document.createElement('input');
    on.type = 'checkbox';
    on.id = `fx-${id}-on`;
    on.className = 'fx-on';
    on.hidden = true;
    on.addEventListener('change', () => {
      this.model.setEnabled(id, on.checked);
      card.classList.toggle('on', on.checked);
      foot.setAttribute('aria-pressed', String(on.checked));
      foot.textContent = on.checked ? 'On' : 'Off';
    });
    this.toggles.set(id, on);

    const head = document.createElement('div');
    head.className = 'fx-head';
    const led = document.createElement('span');
    led.className = 'fx-led';
    led.setAttribute('aria-hidden', 'true');
    const title = document.createElement('b');
    title.className = 'fx-name';
    title.textContent = spec.name;
    head.append(title, led);
    head.title = spec.blurb;

    const params = document.createElement('div');
    params.className = 'fx-params';
    for (const q of spec.params) {
      const knob = createKnob({
        id: `fx-${id}-${q.id}`,
        ariaLabel: `${spec.name} ${q.label}`,
        label: q.label,
        spec: q,
        format: (v) => formatParam(q, v),
        onValue: (v) => this.model.setParam(id, q.id, v),
      });
      params.append(knob.cell);
      this.rows.push({ id, param: q.id, knob });
    }

    const foot = document.createElement('button');
    foot.type = 'button';
    foot.className = 'fx-foot';
    foot.setAttribute('aria-pressed', 'false');
    foot.textContent = 'Off';
    foot.setAttribute('aria-label', `${spec.name} on/off`);
    foot.title = `${spec.name}: click to switch on or off`;
    foot.addEventListener('click', () => {
      on.checked = !on.checked;
      on.dispatchEvent(new Event('change'));
    });
    foot.addEventListener('pointerup', () => foot.blur());

    card.append(head, params, foot, on);
    this.cards.set(id, card);
    this.grid.append(card);
  }

  private refreshAll(): void {
    for (const e of EFFECTS) {
      const on = this.model.state.enabled[e.id];
      const t = this.toggles.get(e.id);
      if (t) t.checked = on;
      this.cards.get(e.id)?.querySelector('.fx-foot')?.setAttribute('aria-pressed', String(on));
      const f = this.cards.get(e.id)?.querySelector('.fx-foot');
      if (f) f.textContent = on ? 'On' : 'Off';
      this.cards.get(e.id)?.classList.toggle('on', on);
    }
    for (const r of this.rows) {
      const v = this.model.state.values[r.id][r.param];
      r.knob.show(v);
    }
  }
}
