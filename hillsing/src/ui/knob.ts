// A rotary knob: drag up or down (Shift for fine), scroll, arrow keys, double-click to reset. A hidden range input
// holds the value (so scripts and tests can set it with an `input` event); the knob is its face.

import { sliderToValue, valueToSlider, type ParamSpec } from '../audio/fxparams';

const SLIDER_STEPS = 1000;

export interface KnobOptions {
  /** Id of the hidden range input. */
  id: string;
  ariaLabel: string;
  label: string;
  spec: ParamSpec;
  format: (value: number) => string;
  hint?: string;
  /** Called with the new value when the user (or a script, through the input) changes it. */
  onValue: (value: number) => void;
}

export interface Knob {
  cell: HTMLElement;
  /** Show `value` without calling onValue (the model changed: a preset, a reset). */
  show(value: number): void;
}

export function createKnob(o: KnobOptions): Knob {
  const q = o.spec;
  const cell = document.createElement('div');
  cell.className = 'fx-param';
  const input = document.createElement('input');
  input.type = 'range';
  input.id = o.id;
  input.hidden = true;
  input.min = '0';
  input.max = String(SLIDER_STEPS);
  input.step = '1';
  const out = document.createElement('output');
  const knob = document.createElement('div');
  knob.className = 'fx-knob';
  knob.tabIndex = 0;
  knob.setAttribute('role', 'slider');
  knob.setAttribute('aria-label', o.ariaLabel);
  knob.setAttribute('aria-valuemin', '0');
  knob.setAttribute('aria-valuemax', String(SLIDER_STEPS));
  knob.title = `${o.hint ? `${o.hint}\n` : ''}Drag up or down, scroll, or use the arrow keys. Double-click to reset.`;

  const t = (): number => Number(input.value) / SLIDER_STEPS;
  const render = (): void => {
    knob.style.setProperty('--t', String(t()));
    knob.setAttribute('aria-valuenow', input.value);
    const text = o.format(sliderToValue(q, t()));
    knob.setAttribute('aria-valuetext', text);
    out.textContent = text;
  };
  const setT = (v: number): void => {
    input.value = String(Math.round(Math.min(1, Math.max(0, v)) * SLIDER_STEPS));
    render();
    o.onValue(sliderToValue(q, t()));
  };
  // Dragging: up turns it up. Hold Shift for fine control. The knob lets go of keyboard focus on pointer-up so the
  // computer keys play again.
  knob.addEventListener('pointerdown', (ev) => {
    ev.preventDefault();
    knob.setPointerCapture(ev.pointerId);
    let y = ev.clientY;
    const move = (m: PointerEvent): void => {
      const dy = y - m.clientY;
      y = m.clientY;
      setT(t() + dy / (m.shiftKey ? 600 : 140));
    };
    const up = (): void => {
      knob.removeEventListener('pointermove', move);
      knob.removeEventListener('pointerup', up);
      knob.removeEventListener('pointercancel', up);
      knob.blur();
    };
    knob.addEventListener('pointermove', move);
    knob.addEventListener('pointerup', up);
    knob.addEventListener('pointercancel', up);
  });
  knob.addEventListener('wheel', (ev) => {
    ev.preventDefault();
    setT(t() - Math.sign(ev.deltaY) * (ev.shiftKey ? 0.005 : 0.03));
  }, { passive: false });
  knob.addEventListener('dblclick', () => setT(valueToSlider(q, q.value)));
  knob.addEventListener('keydown', (ev) => {
    const step = ev.shiftKey ? 0.005 : 0.02;
    const to: Record<string, number> = {
      ArrowUp: t() + step, ArrowRight: t() + step, ArrowDown: t() - step, ArrowLeft: t() - step,
      PageUp: t() + 0.1, PageDown: t() - 0.1, Home: 0, End: 1,
    };
    if (ev.key in to) {
      ev.preventDefault();
      setT(to[ev.key]);
    }
  });
  // Anything that sets the hidden slider (scripts, tests) moves the knob too.
  input.addEventListener('input', () => {
    render();
    o.onValue(sliderToValue(q, t()));
  });

  const name = document.createElement('span');
  name.className = 'fx-label';
  name.textContent = o.label;
  cell.append(knob, name, out, input);
  return {
    cell,
    show(value: number): void {
      input.value = String(Math.round(valueToSlider(q, value) * SLIDER_STEPS));
      render();
    },
  };
}
