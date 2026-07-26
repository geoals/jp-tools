# Attributing a voiceline to the line it belongs to

**Status:** the window bounds are implemented; the onset and duration gates are
proposed and blocked on measurement (`vn-mine/vn-calibrate.py`).

## The problem

`vn-capture.sh` cuts a clip out of the ring buffer and attaches it to a card.
Which audio belongs to the mined line is decided today by a search: take the
window after the line and let VAD find the speech in it. A search returns
*something* almost always, and what it returns is right only as long as the
window contains nothing else.

Two bugs came out of that, both reported from real reading:

- **Wrong anchor.** The window's start was the newest line in `lines.log` at
  the moment the *script* ran, not the line that was mined. On the card-add
  path the script runs seconds later — behind an LLM call, at the time — so
  reading on moved the anchor to the next line. Fixed by stamping the anchor
  when the `addNote` arrives (`VN_ANCHOR_TS`).
- **Unbounded end.** The window ran ten seconds (`VN_MAX_LEN`) forward from the
  line with nothing bounding it, so it swept through however many lines were
  advanced past. On an unvoiced line — whose own span holds no speech to
  prefer — the *next* line's voiceline became the entire clip. Fixed by ending
  the window at the next hooked line.

Both fixes are about the window. Neither makes the capture *know* whether the
line it is mining was voiced at all.

## The signal that would

A hooked line carries more than a timestamp, and two properties of it are
strong enough to check against rather than search within:

1. **A voiced line's audio starts when the line appears.** Textractor hooks the
   text at the moment the engine displays it, and a VN plays the voice on the
   same event. So for a voiced line there is speech beginning at `line_ts`; for
   an unvoiced one there is not, whatever else is in the window. This is the
   test that separates the two cases *by kind* — everything else is a heuristic
   about which of several candidates to prefer.
2. **The line's length predicts its voiceline's length.** Roughly: kana are one
   mora, kanji about two, punctuation none. At a speaking rate this gives an
   expected duration, and a clip far outside it is either two lines merged or
   not this line at all.

## Why this is not implemented yet

Both need a number, and neither number is known:

- The onset gate needs the **spread** of `speech_start − line_ts`, not the
  assumption that it is zero. If some VNs display text a beat before the voice,
  or the hook fires on a different engine event, a gate tight enough to be
  useful would silently drop real audio. It has to be measured, per engine if
  necessary.
- The duration bound needs the spread of **morae per second** across an actual
  cast. Speaking rate varies with character and emotion, the mora estimate
  above is crude, and the bound has to be loose enough never to cut a real
  voiceline — which may leave it too loose to reject anything.

`vn-mine/vn-calibrate.py` measures both against the live ring. It needs
dialogue inside the ring's ~5 minute window: run it during or just after
reading, not while the VN sits on a menu.

## What each check would actually buy

Worth being precise, because the three mechanisms overlap less than they look:

| failure | next-line bound | onset gate | duration bound |
|---|---|---|---|
| unvoiced line, next line voiced, advanced quickly | **fixes** | fixes only if the gate is tighter than the gap | no |
| unvoiced line, no next line yet, sound effect later | no | **fixes** | partly |
| voiced line, advanced fast, next line's voice merged in | **fixes** | no | **fixes** |
| voiced line, correct clip | — | — | — |

The onset gate is the only one that helps when the hotkey is pressed with the
line still on screen, since there is no next line to bound anything with. The
duration bound is the only one that catches a merge where both lines are the
mined line's own speaker.

## Design sketch

Once the numbers exist:

- Call `vn-vad.py --segments` rather than the first/last boundary, so the
  individual utterances are visible instead of pre-merged. Its segment mode
  currently forces its own `min_speech`/`merge_gap`; those would need to become
  parameters.
- Keep the utterance whose start falls within the measured onset gate of
  `line_ts`. No utterance there means the line was not voiced: screenshot only,
  which is already a first-class outcome.
- Extend through following utterances only while the running duration stays
  inside the mora-derived bound. That is what stops a merge at the point where
  the next line's voice would push the clip past any plausible length for this
  line's text.
- Both gates configurable and both able to be switched off, since a VN whose
  engine behaves differently should degrade to today's behaviour rather than
  lose its audio.
