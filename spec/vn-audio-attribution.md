# Attributing a voiceline to the line it belongs to

**Status 2026-07-26.** Two window bugs are fixed and shipped. A third,
narrower one is known and not fixed. The rule that would replace the whole
approach is designed but blocked on data, and `vn-mine/vn-calibrate.py
--collect` is the thing that unblocks it.

Read this before touching the audio window in `vn-mine/vn-capture.sh`.

## The problem

`vn-capture.sh` cuts a clip out of the ring buffer and attaches it to a card.
Which audio belongs to the mined line is decided by a *search*: take the window
after the line and let VAD find the speech in it. A search returns something
almost always, and what it returns is right only while the window contains
nothing else.

## How the VN actually behaves

Established from reading, not from the code — this is the model everything below
rests on:

- A voiced line's audio starts **when the line is hooked**. Textractor fires on
  the engine's display event and the voice plays on the same event.
- Reading faster than the voice does **not** always stop the voice. If the next
  line is **unvoiced**, the previous line's audio keeps playing straight through
  it. If the next line **is voiced**, it interrupts: the previous voice stops and
  the new one starts immediately.
- In practice, when a line is actually *mined*, the voice has almost always
  finished — looking a word up and pressing add takes longer than a voiceline.
  This is why the unfixed case below is rare in real use.

## Fixed

- **Wrong anchor** (`b42a284`). The window's start was the newest line in
  `lines.log` when the *script* ran, not the line that was mined. On the
  card-add path the script runs seconds later, so reading on moved the anchor
  onto the next line. The proxy now stamps the moment the `addNote` arrives and
  passes `VN_ANCHOR_TS`; the script takes the newest line at or before it.
- **Unbounded end** (`2ff501e`). The window ran `VN_MAX_LEN` (10s) forward with
  nothing bounding it, sweeping through however many lines were advanced past.
  On an *unvoiced* line — whose own span holds no speech to prefer — the next
  line's voiceline became the entire clip. The window now ends at the next
  hooked line, and if that leaves under `VN_MIN_LEN` (0.6s) the capture is
  screenshot-only.

## Known and NOT fixed

The next-line bound is a hard cut, and the behaviour model says that is wrong in
one case: **next line unvoiced, previous voice still playing.** The audio
legitimately continues past the next line's timestamp, and the hard cut
truncates it. Rare in practice (see above), and a truncated clip of the right
line is still better than a whole clip of the wrong one, which is why it shipped
that way — but it is a real defect, not a design choice to preserve.

## The rule that fixes it properly

The interruption model gives a calibration-free rule, if segments are visible
instead of pre-merged. Over `vn-vad.py --segments`, in clip-relative time, with
`boundary = next_line_ts - clip_start`:

```
for each segment in order:
    if |seg_start - boundary| <= ONSET_TOL:   stop      # the next line's own voice
    if seg_start < boundary:                  keep      # ours, plainly
    if kept and seg_start - last_kept_end < MERGE_GAP:
                                              keep      # ours, continuing past
                                                        # an unvoiced next line
    else:                                     stop
clip = [first kept start, last kept end]   # empty -> screenshot only
```

Everything the model says falls out: an interrupting voice starts *at* the next
line's hook and is rejected by the first test; a continuing voice has no onset
there and survives via the third. The extraction window goes back to
`line_ts + VN_MAX_LEN` and the hard cut disappears.

Two blockers, both real:

1. `ONSET_TOL` is a measured number, not a guess — see below.
2. `vn-vad.py --segments` currently forces its own `min_speech` (0.1) and
   `merge_gap` (0.15), tuned for `vn-trim.py`'s sentence cuts. They need to
   become parameters, or the coarse sound-effect rejection is lost.

## The measurements

`vn-mine/vn-calibrate.py` scores lines against the speech VAD finds in the ring.
Run it **while reading** — idle on a menu gives a loud ring and no speech.

```sh
vn-mine/vn-calibrate.py                    # one pass over the current ring
vn-mine/vn-calibrate.py --collect          # accumulate across a session
vn-mine/vn-calibrate.py --summarize FILE   # percentiles + onset histogram
```

`--collect` samples every 210s into
`~/.local/share/jp-tools/vn-onset-calibration.tsv`, resumable across sessions.
Overlapping windows are deliberate: a line at the edge of one ring has its voice
cut off by the ring's own edge and reads as silence — the exact quantity being
counted — so a later pass that sees it mid-ring replaces it.

### What one 5-minute ring already showed (n=11 matched lines)

- **Onset clusters hard at zero.** Median −0.07s, stable across every merge
  setting tried. The tight group runs −0.35 … +0.19. This is the finding that
  makes the whole approach viable.
- **But three of eleven sat at +1.87, +2.16, +2.29s**, surviving every merge
  setting. Unexplained. If that is a real mode, a gate tight enough to be useful
  drops a quarter of real audio — silently, which is worse than the bug being
  fixed. **This is the open question `--collect` exists to answer.**
- **The mora model works** once the merge gap is right. Median speaking rate is
  8.4 morae/s at a 1.0s merge — where Japanese speech actually sits. At 0.35s it
  reads 18, because a delivered line splits on its own internal pauses and only
  the first fragment gets measured.

### Two traps this tool already fell into — do not reintroduce

- **Every line searching independently.** Unvoiced lines claim the next line's
  voice, which is the capture bug reproduced inside the measuring tool: it
  reports silent lines as voiced and invents rates like 131 morae/s. Fixed by
  one-to-one assignment — each utterance owned by the last line hooked before
  it. The tell is duplicate `dur` values on neighbouring rows.
- **Circular statistics.** Selecting the sample by `|onset| < 1.0` and then
  reporting that onsets fall within 1.0. The raw column is the evidence; no
  voiced/unvoiced threshold is applied before the summary.

## Which mechanism covers which failure

They overlap less than they look:

| failure | next-line bound | onset gate | duration bound |
|---|---|---|---|
| unvoiced line, next line voiced, advanced fast | **fixes** | only if tighter than the gap | no |
| unvoiced line, no next line yet, sound effect later | no | **fixes** | partly |
| voiced line, advanced fast, next line's voice merged in | **fixes** | no | **fixes** |
| voiced line, next line unvoiced, voice still playing | **breaks it** | no | no |

The onset gate is the only one that helps when the hotkey is pressed with the
line still on screen — there is no next line to bound with. The duration bound
is the only one that catches a merge where both lines share a speaker.

## Next steps, in order

1. Run `--collect` across a real session (hundreds of lines, not eleven).
2. `--summarize` it. The histogram answers the one question: is the onset
   distribution one cluster or two? If one, `ONSET_TOL` is its p95 and the rule
   above can be implemented. If two, find out what the +2s lines have in common
   before gating anything.
3. Parameterise `vn-vad.py --segments` thresholds.
4. Implement the rule, delete the hard cut, keep `VN_MIN_LEN` as the
   screenshot-only fallback.
5. Both gates configurable and switchable off — a VN whose engine behaves
   differently should degrade to today's behaviour rather than lose its audio.
