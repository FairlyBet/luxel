import type { Entity, Scene } from "./luxel.js";

export interface OrbitOptions {
  /** Radians per canvas height dragged (default 4). */
  rotateSpeed: number;
  /** Zoom per wheel unit / pinch, as a factor of the default (1). */
  zoomSpeed: number;
  /** Distance moved per canvas height dragged, as a share of the distance (1). */
  panSpeed: number;
  /** Right-drag, shift-drag and two fingers pan (true). */
  enablePan: boolean;
  /** Share of the motion applied each 60th of a second, 0–1; 0 or 1: none
   *  carried (0.25). */
  damping: number;
  minDistance: number;
  maxDistance: number;
  /** Radians above the horizon, limits (±1.5). */
  minPitch: number;
  maxPitch: number;
  /** Radians per second around the target; 0: off. */
  autoRotate: number;
}

/** An orbit camera: drag to turn, wheel / pinch to zoom, right-drag /
 *  shift-drag / two fingers to pan, a tap reported for picking. Zoom moves
 *  the camera (a perspective one); an orthographic camera's zoom is its
 *  `ymag` (`scene.setCamera`). */
export class OrbitControls implements OrbitOptions {
  constructor(scene: Scene, camera: Entity, element: HTMLElement, options?: Partial<OrbitOptions>);
  scene: Scene;
  camera: Entity;
  element: HTMLElement;
  /** The point it turns around. */
  target: [number, number, number];
  /** Distance from the target. */
  distance: number;
  /** Radians around the vertical axis (0: looking down −Z). */
  yaw: number;
  /** Radians above the horizon. */
  pitch: number;
  /** Called with a tap's normalized device coordinates (`scene.pick`'s). */
  onTap: ((x: number, y: number) => void) | null;
  rotateSpeed: number;
  zoomSpeed: number;
  panSpeed: number;
  enablePan: boolean;
  damping: number;
  minDistance: number;
  maxDistance: number;
  minPitch: number;
  maxPitch: number;
  autoRotate: number;
  /** Looks at `root` (`scene.frameBounds`) from the current side; false if
   *  there was nothing to frame. */
  frame(root: Entity, margin?: number): boolean;
  /** Applies input and easing `dt` seconds on and places the camera; true if
   *  it moved. */
  update(dt: number, force?: boolean): boolean;
  /** Where the camera is. */
  eye(): [number, number, number];
  /** Stops listening to the element. */
  dispose(): void;
}
