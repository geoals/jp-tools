// The slice of Markdown the model emits, parsed into a plain tree: paragraphs,
// `-`/`*` bullets, `**bold**` and `*italic*`.
//
// Shared because it is a claim about what the *model* writes, and both hosts
// read the same `/api/reader/explain`. The rendering is not shared: kotodex-server
// is Preact and builds vnodes, the overlay is a plain page and builds DOM. That
// split is also what keeps this safe — neither host ever assembles HTML from
// model output, so there is no innerHTML seam to escape.

/** `**bold**` and `*italic*` runs in one line, everything else literal.
 *  Each span is `{ text, style }` with `style` one of null, "bold", "italic". */
function spans(text) {
  const out = [];
  const re = /\*\*([^*]+)\*\*|\*([^*]+)\*/g;
  let last = 0;
  let m;
  while ((m = re.exec(text))) {
    if (m.index > last) out.push({ text: text.slice(last, m.index), style: null });
    if (m[1] != null) out.push({ text: m[1], style: "bold" });
    else out.push({ text: m[2], style: "italic" });
    last = re.lastIndex;
  }
  if (last < text.length) out.push({ text: text.slice(last), style: null });
  return out;
}

/** Blocks of `{ type: "p", spans }` or `{ type: "ul", items: [spans] }`. */
export function parseMarkdown(src) {
  return (src || "")
    .trim()
    .split(/\n{2,}/)
    .map((block) => {
      const rows = block.split("\n");
      if (rows.length && rows.every((l) => /^\s*[-*]\s+/.test(l))) {
        return { type: "ul", items: rows.map((l) => spans(l.replace(/^\s*[-*]\s+/, ""))) };
      }
      // Soft-wrapped lines in one block are one paragraph.
      return { type: "p", spans: spans(rows.join(" ")) };
    });
}
