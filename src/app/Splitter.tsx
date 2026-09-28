import type { KeyboardEvent, PointerEvent } from "react";

interface Props {
  /** `x` resizes a width (a vertical bar), `y` a height (a horizontal bar). */
  axis: "x" | "y";
  size: number;
  onResize: (size: number) => void;
  /** Dragging towards the start grows the panel (a panel below or to the right). */
  inverted?: boolean;
  label: string;
}

const KEY_STEP = 24;

/** A draggable, keyboard-adjustable divider between two panels. */
export function Splitter({ axis, size, onResize, inverted = false, label }: Props) {
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const bar = event.currentTarget;
    const start = axis === "x" ? event.clientX : event.clientY;
    bar.setPointerCapture(event.pointerId);
    document.body.classList.add(axis === "x" ? "resizing-x" : "resizing-y");
    const move = (e: globalThis.PointerEvent) => {
      const delta = (axis === "x" ? e.clientX : e.clientY) - start;
      onResize(size + (inverted ? -delta : delta));
    };
    const stop = () => {
      bar.removeEventListener("pointermove", move);
      bar.removeEventListener("pointerup", stop);
      bar.removeEventListener("pointercancel", stop);
      document.body.classList.remove("resizing-x", "resizing-y");
    };
    bar.addEventListener("pointermove", move);
    bar.addEventListener("pointerup", stop);
    bar.addEventListener("pointercancel", stop);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const grow = axis === "x" ? "ArrowRight" : "ArrowDown";
    const shrink = axis === "x" ? "ArrowLeft" : "ArrowUp";
    if (event.key !== grow && event.key !== shrink) return;
    event.preventDefault();
    const step = (event.key === grow) !== inverted ? KEY_STEP : -KEY_STEP;
    onResize(size + step);
  };

  return (
    <div
      className={`splitter splitter-${axis}`}
      role="separator"
      aria-label={label}
      aria-orientation={axis === "x" ? "vertical" : "horizontal"}
      aria-valuenow={size}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
    />
  );
}
