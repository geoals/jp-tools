// Small reusable controls. Anything here is used by more than one panel; a
// control with a single caller belongs in that panel's own file.

import { html } from "htm/preact";

/** A row of mutually exclusive choices, styled as one segmented control. */

export function SegmentedControl({ value, options, onChange, label }) {
  return html`
    <div class="segmented" role="group" aria-label=${label}>
      ${options.map(
        (o) => html`
          <button
            type="button"
            class=${o.value === value ? "segment segment-on" : "segment"}
            aria-pressed=${o.value === value}
            onClick=${() => onChange(o.value)}
          >
            ${o.label}
          </button>
        `,
      )}
    </div>
  `;
}
