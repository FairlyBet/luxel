// An orbit camera for a scene: drag to turn around a target, wheel or pinch
// to zoom, right-drag / shift-drag / two fingers to pan, a tap reported for
// picking. Moves ease out (damping). Ships beside the package:
//
//   import { OrbitControls } from "@fairlybet/luxel/orbit";
//   const orbit = new OrbitControls(scene, camera, canvas);
//   orbit.frame(root);                     // look at a model
//   orbit.onTap = (x, y) => scene.pick(camera, x, y).then(...);
//   // each frame, before needsRender / renderFrame:
//   orbit.update(dt);
//
// It only calls `scene.lookAt` (and `frameBounds` in `frame`), so a still
// orbit leaves the scene unchanged: `needsRender` stays false.

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

export class OrbitControls {
  /**
   * @param {import("./luxel.js").Scene} scene
   * @param {number} camera the camera entity it moves
   * @param {HTMLElement} element where pointer and wheel input comes from
   * @param {Partial<OrbitOptions>} [options]
   */
  constructor(scene, camera, element, options = {}) {
    this.scene = scene;
    this.camera = camera;
    this.element = element;
    /** The point it turns around. */
    this.target = [0, 0, 0];
    /** Distance from the target. */
    this.distance = 3;
    /** Radians around the vertical axis (0: looking down −Z). */
    this.yaw = 0;
    /** Radians above the horizon. */
    this.pitch = 0.2;
    Object.assign(this, DEFAULTS, options);
    /** Called with a tap's normalized device coordinates (`scene.pick`'s). */
    this.onTap = null;
    this._v = { yaw: 0, pitch: 0, zoom: 0, panX: 0, panY: 0 };
    this._pointers = new Map();
    this._moved = 0;
    this._pinch = 0;
    this._listen();
  }

  /**
   * Looks at `root` (`scene.frameBounds`) from the current side: the target
   * at its center, the distance that fits it, the zoom limits around it.
   * False if there was nothing to frame.
   */
  frame(root, margin = 1.1) {
    this.update(0, true); // the camera where the controls say first
    const f = this.scene.frameBounds(this.camera, root, margin);
    if (f.length < 4) return false;
    this.target = [f[0], f[1], f[2]];
    this.distance = f[3];
    this.minDistance = f[3] * 0.05;
    this.maxDistance = f[3] * 20;
    this._v = { yaw: 0, pitch: 0, zoom: 0, panX: 0, panY: 0 };
    return true;
  }

  /**
   * Applies input and its easing, `dt` seconds on, and places the camera;
   * true if it moved. `force` places it even when nothing moved.
   */
  update(dt, force = false) {
    const v = this._v;
    const moving = Object.values(v).some((x) => Math.abs(x) > 1e-6);
    if (!moving && !this.autoRotate && !force) return false;
    // The spin at its own speed, beside whatever the user does.
    if (this.autoRotate) this.yaw += this.autoRotate * dt;
    // A damped step: the share of the velocity applied this frame, the rest
    // carried (frame-rate independent).
    const damping = Math.min(1, Math.max(0, this.damping));
    const keep = damping > 0 && damping < 1 ? Math.pow(1 - damping, dt * 60) : 0;
    const step = 1 - keep;
    this.yaw += v.yaw * step;
    this.pitch = clamp(this.pitch + v.pitch * step, this.minPitch, this.maxPitch);
    this.distance = clamp(this.distance * Math.exp(v.zoom * step), this.minDistance, this.maxDistance);
    if (v.panX || v.panY) {
      // In the view plane, scaled by the distance (the target keeps under
      // the pointer, roughly).
      const s = this.distance * step;
      const [cy, sy, cp, sp] = [Math.cos(this.yaw), Math.sin(this.yaw), Math.cos(this.pitch), Math.sin(this.pitch)];
      const right = [cy, 0, -sy];
      const up = [-sp * sy, cp, -sp * cy];
      for (let i = 0; i < 3; i++) this.target[i] += (right[i] * -v.panX + up[i] * v.panY) * s;
    }
    for (const k in v) v[k] = Math.abs(v[k] * keep) > 1e-6 ? v[k] * keep : 0;
    this.scene.lookAt(this.camera, this.eye(), this.target);
    return true;
  }

  /** Where the camera is: `[x, y, z]`. */
  eye() {
    const { yaw, pitch, distance, target } = this;
    return [
      target[0] + distance * Math.cos(pitch) * Math.sin(yaw),
      target[1] + distance * Math.sin(pitch),
      target[2] + distance * Math.cos(pitch) * Math.cos(yaw),
    ];
  }

  /** Stops listening to the element. */
  dispose() {
    for (const [type, f, o] of this._listeners) this.element.removeEventListener(type, f, o);
    this._listeners = [];
  }

  _listen() {
    const el = this.element;
    el.style.touchAction = "none";
    const ndc = (e) => {
      const r = el.getBoundingClientRect();
      return [((e.clientX - r.left) / r.width) * 2 - 1, 1 - ((e.clientY - r.top) / r.height) * 2];
    };
    const down = (e) => {
      el.setPointerCapture?.(e.pointerId);
      this._pointers.set(e.pointerId, { x: e.clientX, y: e.clientY, pan: e.button === 2 || e.shiftKey });
      // A second finger makes it a gesture, never a tap.
      this._moved = this._pointers.size === 1 ? 0 : Infinity;
      if (this._pointers.size === 2) this._pinch = this._spread();
    };
    const move = (e) => {
      const p = this._pointers.get(e.pointerId);
      if (!p) return;
      const h = el.clientHeight || 1;
      const dx = (e.clientX - p.x) / h;
      const dy = (e.clientY - p.y) / h;
      p.x = e.clientX;
      p.y = e.clientY;
      this._moved += Math.abs(dx) + Math.abs(dy);
      const v = this._v;
      if (this._pointers.size === 1) {
        if (p.pan && this.enablePan) {
          v.panX += dx * this.panSpeed;
          v.panY += dy * this.panSpeed;
        } else {
          v.yaw -= dx * this.rotateSpeed;
          v.pitch += dy * this.rotateSpeed;
        }
      } else if (this._pointers.size === 2) {
        const spread = this._spread();
        if (this._pinch > 0 && spread > 0) v.zoom += Math.log(this._pinch / spread) * this.zoomSpeed;
        this._pinch = spread;
        // Both fingers moving together: a pan, at half each one's share.
        if (this.enablePan) {
          v.panX += (dx / 2) * this.panSpeed;
          v.panY += (dy / 2) * this.panSpeed;
        }
      }
    };
    const up = (e) => {
      const single = this._pointers.size === 1;
      this._pointers.delete(e.pointerId);
      if (this._pointers.size < 2) this._pinch = 0;
      if (single && e.type === "pointerup" && this._moved < 0.01 && this.onTap) this.onTap(...ndc(e));
    };
    const wheel = (e) => {
      e.preventDefault();
      const lines = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? 400 : 1;
      this._v.zoom += e.deltaY * lines * 0.001 * this.zoomSpeed;
    };
    const menu = (e) => this.enablePan && e.preventDefault();
    this._listeners = [
      ["pointerdown", down, undefined],
      ["pointermove", move, undefined],
      ["pointerup", up, undefined],
      ["pointercancel", up, undefined],
      ["wheel", wheel, { passive: false }],
      ["contextmenu", menu, undefined],
    ];
    for (const [type, f, o] of this._listeners) el.addEventListener(type, f, o);
  }

  _spread() {
    const [a, b] = [...this._pointers.values()];
    return a && b ? Math.hypot(a.x - b.x, a.y - b.y) : 0;
  }
}

/** @typedef {typeof DEFAULTS} OrbitOptions */
const DEFAULTS = {
  /** Radians per canvas height dragged. */
  rotateSpeed: 4,
  /** Zoom per wheel unit / pinch, as a factor of the default. */
  zoomSpeed: 1,
  /** Distance moved per canvas height dragged, as a share of the distance. */
  panSpeed: 1,
  enablePan: true,
  /** Share of the motion applied each 60th of a second (1: none carried). */
  damping: 0.25,
  minDistance: 0.01,
  maxDistance: 1e6,
  minPitch: -1.5,
  maxPitch: 1.5,
  /** Radians per second around the target; 0: off. */
  autoRotate: 0,
};
