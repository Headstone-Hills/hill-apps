import { describe, expect, it } from 'vitest';
import { ControlMap, describeMidi } from '../src/midi/controls';

const memory = () => {
  const m = new Map<string, string>();
  return { getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
};

describe('controller mapping', () => {
  it('does nothing until a target is armed, then makes the next moving controller its source', () => {
    const c = new ControlMap(null);
    expect(c.handle({ type: 'cc', controller: 74, value: 80 })).toBeNull();
    c.learn('pitch');
    expect(c.armedTarget).toBe('pitch');
    const first = c.handle({ type: 'cc', controller: 74, value: 100 });
    expect(first).not.toBeNull();
    expect(first!.target).toBe('pitch');
    expect(first!.value).toBeCloseTo(36 / 63, 6);
    expect(c.armedTarget).toBeNull();
    expect(c.controllerFor('pitch')).toBe(74);
    expect(c.handle({ type: 'cc', controller: 75, value: 5 })).toBeNull(); // another knob is not mapped
  });

  it('maps a knob to pitch (centre at 64, full range both ways) and to modulation (0 to 1)', () => {
    const c = new ControlMap(null);
    c.learn('pitch');
    c.handle({ type: 'cc', controller: 20, value: 64 });
    c.learn('mod');
    c.handle({ type: 'cc', controller: 21, value: 0 });
    expect(c.handle({ type: 'cc', controller: 20, value: 64 })!.value).toBe(0);
    expect(c.handle({ type: 'cc', controller: 20, value: 127 })!.value).toBe(1);
    expect(c.handle({ type: 'cc', controller: 20, value: 0 })!.value).toBe(-1);
    expect(c.handle({ type: 'cc', controller: 21, value: 127 })).toEqual({ target: 'mod', value: 1 });
    expect(c.handle({ type: 'cc', controller: 21, value: 0 })!.value).toBe(0);
  });

  it('ignores the sustain pedal while learning, and one controller keeps only one job', () => {
    const c = new ControlMap(null);
    c.learn('mod');
    expect(c.handle({ type: 'cc', controller: 64, value: 127 })).toBeNull();
    expect(c.armedTarget).toBe('mod'); // still waiting
    c.handle({ type: 'cc', controller: 30, value: 10 });
    c.learn('pitch');
    c.handle({ type: 'cc', controller: 30, value: 10 }); // the same knob moves over to pitch
    expect(c.controllerFor('pitch')).toBe(30);
    expect(c.controllerFor('mod')).toBeUndefined();
  });

  it('remembers its mapping, and can be cleared', () => {
    const store = memory();
    const c = new ControlMap(store);
    c.learn('pitch');
    c.handle({ type: 'cc', controller: 12, value: 64 });
    expect(new ControlMap(store).controllerFor('pitch')).toBe(12);
    c.clear('pitch');
    expect(new ControlMap(store).controllerFor('pitch')).toBeUndefined();
  });

  it('describes events for the monitor', () => {
    expect(describeMidi({ type: 'cc', controller: 74, value: 64 })).toBe('controller CC 74 = 64');
    expect(describeMidi({ type: 'pitchBend', value: 0.5 })).toBe('pitch bend +0.50');
    expect(describeMidi({ type: 'noteOn', note: 60, velocity: 1 })).toContain('note 60');
  });
});
