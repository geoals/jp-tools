# Yomitan Integration

Yomitan is the primary popup dictionary and should remain so. The question is
how to connect it to the knowledge DB.

## Option A: Yomitan Fork

**Approach:** Fork Yomitan and add hooks that report lookups and card-creation
events to our local API.

**Pros:**
- Full control over what data is captured and when
- Can add UI elements (e.g. word status badge in the popup)
- Can intercept the "add to Anki" flow to also update our DB

**Cons:**
- Maintenance burden — must keep up with upstream Yomitan updates
- Browser extension development is its own ecosystem of pain
- Yomitan's codebase is large and non-trivial to modify

**Feasibility:** Medium. Viable if the fork is minimal (a few hook points rather
than deep changes). Risk increases over time as upstream diverges.

**Status:** This option was investigated and found to require forking. Remains
viable if the fork scope is kept small.

---

## Option B: Companion Browser Extension

**Approach:** A separate, small browser extension that:
1. Observes the texthooker page DOM for Yomitan popup events
2. Captures which words are looked up (by watching for Yomitan's popup elements)
3. Sends lookup events to our local API
4. Optionally intercepts AnkiConnect requests from Yomitan to also log card
   creation events

**Pros:**
- No Yomitan fork needed
- Small, focused extension that's easy to maintain
- Can also enhance the texthooker page (inject highlighting CSS, etc.)

**Cons:**
- Fragile — depends on Yomitan's DOM structure, which can change between versions
- Intercepting AnkiConnect (via webRequest API on localhost:8765) may have
  browser permission issues
- Can't modify the Yomitan popup itself

**Feasibility:** Medium. The DOM observation approach is brittle. The
AnkiConnect interception is more robust but needs careful permission handling.

---

## Option C: AnkiConnect Proxy

**Approach:** Run a proxy on localhost that sits between Yomitan and Anki:
- Yomitan sends requests to our proxy (configure Yomitan's AnkiConnect URL)
- Proxy logs the card data to our DB
- Proxy forwards the request to the real AnkiConnect

**Pros:**
- No Yomitan modification at all
- Clean interception of exactly what gets sent to Anki
- Also captures the sentence, reading, definition — everything Yomitan sends

**Cons:**
- Only captures card-creation events, not lookups (you look up many words you
  don't mine)
- Yomitan's AnkiConnect URL must be reconfigured (minor)
- Must handle all AnkiConnect protocol messages, not just addNote

**Feasibility:** High. This is a simple HTTP proxy. The AnkiConnect protocol is
JSON-RPC and well-documented.

---

## Option D: Minimal Integration (Recommended Starting Point)

**Approach:** Don't deeply integrate with Yomitan at all. Instead:
1. Our texthooker page handles highlighting independently (tokenizer + DB)
2. Yomitan works on top of the page as it always does
3. Word status updates happen through our own UI (click a highlighted word →
   mark as known/learning)
4. Card mining happens through our own UI (click to mine → captures sentence,
   word, screenshot, audio)
5. Anki import/export is a separate batch operation

**Pros:**
- Zero coupling to Yomitan internals
- Simplest to build and maintain
- Yomitan upgrades never break anything
- Clear separation: Yomitan = dictionary lookup, our tool = knowledge tracking

**Cons:**
- Duplicated effort: if you mine a card in Yomitan AND our tool, you do it twice
- No automatic "looked up in Yomitan → mark as seen" tracking

**Feasibility:** High. This is the pragmatic starting point.

---

## Recommendation

Start with **Option D** to get something working. Evaluate whether the lack of
lookup tracking is actually a problem in practice. If it is, add **Option C**
(AnkiConnect proxy) as a second step — it's the least invasive way to capture
card creation events. Consider **Option A** (fork) only if deeper integration
proves necessary and the fork scope can be kept minimal.
