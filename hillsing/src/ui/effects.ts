// The Effects section: a card per effect (on/off switch + sliders), a preset menu and a reset button. It only edits
// the FxModel; the audio rack follows the model (see audio/bus.ts), so the panel works before audio has started.

import { EFFECTS, FX_PRESETS, formatParam, sliderToValue, valueToSlider, type EffectId, type FxModel } from '../audio/fxparams';

const SLIDER_STEPS = 1000;

interface Row {
  id: EffectId;
  param: string;
  input: HTMLInputElement;
  out: HTMLOutputElement;
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

    const head = document.createElement('div');
    head.className = 'fx-head';
    const label = document.createElement('label');
    label.className = 'fx-toggle';
    const on = document.createElement('input');
    on.type = 'checkbox';
    on.id = `fx-${id}-on`;
    on.addEventListener('change', () => {
      this.model.setEnabled(id, on.checked);
      card.classList.toggle('on', on.checked);
      on.blur();
    });
    const title = document.createElement('b');
    title.textContent = spec.name;
    label.append(on, title);
    const blurb = document.createElement('span');
    blurb.className = 'hint';
    blurb.textContent = spec.blurb;
    head.append(label, blurb);
    card.append(head);
    this.toggles.set(id, on);

    const params = document.createElement('div');
    params.className = 'fx-params';
    for (const q of spec.params) {
      const wrap = document.createElement('label');
      wrap.className = 'fx-param';
      wrap.htmlFor = `fx-${id}-${q.id}`;
      const name = document.createElement('span');
      name.textContent = q.label;
      const out = document.createElement('output');
      const input = document.createElement('input');
      input.type = 'range';
      input.id = `fx-${id}-${q.id}`;
      input.min = '0';
      input.max = String(SLIDER_STEPS);
      input.step = '1';
      input.addEventListener('input', () => {
        const v = sliderToValue(q, Number(input.value) / SLIDER_STEPS);
        out.textContent = formatParam(q, v);
        this.model.setParam(id, q.id, v);
      });
      input.addEventListener('pointerup', () => input.blur()); // so the computer keys play again
      const top = document.createElement('span');
      top.className = 'fx-param-top';
      top.append(name, out);
      wrap.append(top, input);
      params.append(wrap);
      this.rows.push({ id, param: q.id, input, out });
    }
    card.append(params);
    this.cards.set(id, card);
    this.grid.append(card);
  }

  private refreshAll(): void {
    for (const e of EFFECTS) {
      const on = this.model.state.enabled[e.id];
      const t = this.toggles.get(e.id);
      if (t) t.checked = on;
      this.cards.get(e.id)?.classList.toggle('on', on);
    }
    for (const r of this.rows) {
      const spec = EFFECTS.find((e) => e.id === r.id)!.params.find((q) => q.id === r.param)!;
      const v = this.model.state.values[r.id][r.param];
      r.input.value = String(Math.round(valueToSlider(spec, v) * SLIDER_STEPS));
      r.out.textContent = formatParam(spec, v);
    }
  }
}
