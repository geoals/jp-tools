// A dialog over the page, for a form that would otherwise push the card's
// content around while it is open.
//
// Escape and a click on the backdrop close it: a dialog with only an in-form
// cancel button traps anyone who opened it by mistake. The page behind is locked
// while it is open, because a half-filled form over a scrolling shelf reads as
// two places to look.

import { html } from "htm/preact";
import { useEffect } from "preact/hooks";

export function Modal({ title, onClose, children }) {
  useEffect(() => {
    const onKey = (e) => e.key === "Escape" && onClose();
    document.addEventListener("keydown", onKey);
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.removeEventListener("keydown", onKey);
      document.body.style.overflow = prevOverflow;
    };
  }, [onClose]);

  return html`
    <div class="modal-backdrop" onClick=${onClose}>
      <div
        class="modal"
        role="dialog"
        aria-modal="true"
        aria-label=${title}
        onClick=${(e) => e.stopPropagation()}
      >
        <div class="modal-head">
          <h3>${title}</h3>
          <button
            type="button"
            class="ghost modal-close"
            title="Close"
            onClick=${onClose}
          >
            ×
          </button>
        </div>
        <div class="modal-body">${children}</div>
      </div>
    </div>
  `;
}
