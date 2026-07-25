// Small reusable controls. Anything here is used by more than one panel; a
// control with a single caller belongs in that panel's own file.

import { html } from "htm/preact";

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

/* The daily bar chart, over two independent choices: which measure (time or
   characters) and whether to stack it by dialogue.

   Minutes and characters get one chart with a switch rather than two charts,
   because they are the same question — how much reading happened — asked in
   the two units it has. They are deliberately never overlaid: that would need
   a second y-scale, and where two scales line up is a choice rather than a
   fact. Reading speed is exactly the relationship between them and already
   has its own chart. */
