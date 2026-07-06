import { html } from 'htm/preact';
import { useRef } from 'preact/hooks';

/**
 * Drag-a-box crop overlay on top of the photo. Works with mouse and touch
 * (pointer events). `rect` is in displayed pixels relative to the container;
 * the parent converts to fractions when calling the OCR endpoint.
 * `onRelease(rect)` fires when a drag ends with a usable box.
 */
export function CropBox({ src, rect, setRect, onRelease, disabled, containerRef }) {
  const dragStart = useRef(null);

  function relPos(e) {
    const bounds = containerRef.current.getBoundingClientRect();
    return {
      x: Math.min(Math.max(e.clientX - bounds.left, 0), bounds.width),
      y: Math.min(Math.max(e.clientY - bounds.top, 0), bounds.height),
    };
  }

  function onPointerDown(e) {
    if (disabled) return;
    e.preventDefault();
    containerRef.current.setPointerCapture(e.pointerId);
    const p = relPos(e);
    dragStart.current = p;
    setRect({ x: p.x, y: p.y, w: 0, h: 0 });
  }

  function onPointerMove(e) {
    if (disabled || !dragStart.current) return;
    e.preventDefault();
    const p = relPos(e);
    const s = dragStart.current;
    setRect({
      x: Math.min(s.x, p.x),
      y: Math.min(s.y, p.y),
      w: Math.abs(p.x - s.x),
      h: Math.abs(p.y - s.y),
    });
  }

  function onPointerUp(e) {
    if (disabled || !dragStart.current) return;
    const p = relPos(e);
    const s = dragStart.current;
    dragStart.current = null;
    if (Math.abs(p.x - s.x) < 8 || Math.abs(p.y - s.y) < 8) {
      setRect(null); // too small — treat as a tap that clears the box
      return;
    }
    const finalRect = {
      x: Math.min(s.x, p.x),
      y: Math.min(s.y, p.y),
      w: Math.abs(p.x - s.x),
      h: Math.abs(p.y - s.y),
    };
    setRect(finalRect);
    if (onRelease) onRelease(finalRect);
  }

  return html`
    <div
      ref=${containerRef}
      class="crop-container ${disabled ? 'crop-disabled' : ''}"
      onPointerDown=${onPointerDown}
      onPointerMove=${onPointerMove}
      onPointerUp=${onPointerUp}
      onPointerCancel=${() => { dragStart.current = null; }}
    >
      <img src=${src} alt="manga panel" draggable=${false} />
      ${rect && html`
        <div
          class="crop-rect"
          style=${{
            left: `${rect.x}px`,
            top: `${rect.y}px`,
            width: `${rect.w}px`,
            height: `${rect.h}px`,
          }}
        ></div>
      `}
    </div>
  `;
}
