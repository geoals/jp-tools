// `POST /api/reader/explain`, read as it arrives.
//
// Shared because the wire format is one contract and both hosts speak it. It is
// server-sent events over a POST, so `EventSource` — which can only GET —
// cannot be used, and the frames are parsed here off the response body.

/** Stream one explanation, calling `onText` with the answer so far each time it
 *  grows. Resolves with the full text; throws on an error frame or a failed
 *  request, which is the same failure the caller would have got unstreamed. */
export async function streamExplain({ context, focus = "", onText }) {
  const res = await fetch("/api/reader/explain", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ context, focus }),
  });
  // A failure before the stream opens is still an ordinary HTTP error.
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.error ?? res.statusText);
  }

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buf = "";
  let text = "";

  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    buf += decoder.decode(value, { stream: true });
    // Frames are separated by a blank line, and the last piece of the buffer is
    // whatever has not been terminated yet.
    const frames = buf.split("\n\n");
    buf = frames.pop();
    for (const frame of frames) {
      const event = frame.match(/^event:\s*(.*)$/m)?.[1]?.trim();
      const data = frame
        .split("\n")
        .filter((l) => l.startsWith("data:"))
        .map((l) => l.slice(5).trim())
        .join("\n");
      // The text is JSON-encoded because a delta can contain a newline, which
      // is the one thing an SSE `data:` field cannot carry.
      if (event === "delta") {
        text += JSON.parse(data);
        onText?.(text);
      } else if (event === "error") {
        throw new Error(data);
      }
    }
  }
  return text;
}
