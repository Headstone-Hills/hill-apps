// Pin a bar to the bottom of what is actually on screen when the page runs inside a frame that is as tall as its whole
// content (how a viewer may embed it): there, "position: fixed; bottom: 0" pins to the bottom of the *frame*, far below
// the screen. A column of invisible strips is watched with an IntersectionObserver (which reports how much of each strip
// the user can see, even from inside a frame); the bar is placed just above the lowest visible edge.

const STEP = 160; // height of one strip, px

export class ScreenBottomPin {
  private readonly holder = document.createElement('div');
  private readonly visible = new Map<number, number>(); // strip index -> page y of its lowest visible edge
  private observer: IntersectionObserver | null = null;
  private strips = 0;
  private active = false;

  constructor(private readonly bar: HTMLElement) {
    this.holder.setAttribute('aria-hidden', 'true');
    this.holder.style.cssText = 'position:absolute;left:0;top:0;width:1px;opacity:0;pointer-events:none;';
  }

  /** Follow the screen (true) or go back to ordinary fixed positioning (false). */
  setActive(on: boolean): void {
    if (on === this.active) {
      if (on) this.rebuild();
      return;
    }
    this.active = on;
    this.bar.classList.toggle('follow', on);
    if (on) {
      document.body.append(this.holder);
      this.rebuild();
    } else {
      this.observer?.disconnect();
      this.observer = null;
      this.holder.remove();
      this.visible.clear();
      this.bar.style.top = '';
    }
  }

  private rebuild(): void {
    const height = Math.max(document.documentElement.scrollHeight, window.innerHeight);
    const strips = Math.ceil(height / STEP);
    if (strips === this.strips && this.observer) {
      this.place();
      return;
    }
    this.observer?.disconnect();
    this.visible.clear();
    this.strips = strips;
    this.holder.textContent = '';
    this.holder.style.height = `${strips * STEP}px`;
    this.observer = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          const i = Number((e.target as HTMLElement).dataset.i);
          if (e.isIntersecting && e.intersectionRect.height > 0) {
            // intersectionRect is in the strip's own frame, so its bottom edge is a position in this page.
            this.visible.set(i, i * STEP + (e.intersectionRect.bottom - e.boundingClientRect.top));
          } else this.visible.delete(i);
        }
        this.place();
      },
      { threshold: Array.from({ length: 11 }, (_, k) => k / 10) },
    );
    for (let i = 0; i < strips; i++) {
      const s = document.createElement('div');
      s.dataset.i = String(i);
      s.style.cssText = `position:absolute;left:0;top:${i * STEP}px;width:1px;height:${STEP}px;`;
      this.holder.append(s);
      this.observer.observe(s);
    }
  }

  private place(): void {
    if (!this.active || this.visible.size === 0) return;
    const bottom = Math.max(...this.visible.values());
    this.bar.style.top = `${Math.max(0, Math.round(bottom - this.bar.offsetHeight))}px`;
  }
}
