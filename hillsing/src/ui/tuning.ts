// The "Sample voice tuning" panel: one knob per tuning parameter, in both the Studio and Perform views.

import { formatTuning, TUNING_PARAMS, type TuningModel } from '../utau/tuning';
import { createKnob, type Knob } from './knob';

export class TuningPanel {
  private readonly knobs = new Map<string, Knob>();

  constructor(grid: HTMLElement, resetButton: HTMLButtonElement, private readonly model: TuningModel) {
    for (const p of TUNING_PARAMS) {
      const knob = createKnob({
        id: `tune-${p.id}`,
        ariaLabel: `Sample voice ${p.label}`,
        label: p.label,
        spec: p,
        format: (v) => formatTuning(p.id, v),
        hint: p.hint,
        onValue: (v) => model.set(p.id, v),
      });
      grid.append(knob.cell);
      this.knobs.set(p.id, knob);
    }
    resetButton.addEventListener('click', () => {
      model.reset();
      resetButton.blur();
    });
    model.onChange((id) => {
      if (id === null) this.refreshAll();
    });
    this.refreshAll();
  }

  private refreshAll(): void {
    for (const p of TUNING_PARAMS) this.knobs.get(p.id)?.show(this.model.values[p.id]);
  }
}
