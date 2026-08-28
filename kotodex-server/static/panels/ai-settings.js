// Which AI service answers, and the key that opens it.
//
// The key is write-only: `GET /api/settings` never returns it, only
// `llm_has_key`, so this box shows whether one is stored and not what it is. It
// is saved through its own endpoint, which answers whether the key actually
// worked — a key pasted with a character missing is the ordinary mistake here,
// and the alternative to saying so now is a failed explain button three lines
// into a session.
//
// The overlay carries the same three controls under ⚙ → AI. Not shared code:
// this is Preact and that is vanilla DOM over a layer surface, and there is
// nothing between them but the two endpoints.

import { html } from "htm/preact";
import { useState } from "preact/hooks";
import { api } from "../api.js";

/** What each service wants in the two boxes under it. */
const SERVICES = {
  anthropic: {
    label: "Anthropic",
    baseUrl: "https://api.anthropic.com",
    note: "Claude, from console.anthropic.com. Leave the address alone unless you are proxying it.",
  },
  openai: {
    label: "OpenAI-compatible",
    baseUrl: "https://api.openai.com/v1",
    note: "Anything speaking OpenAI's chat API: OpenAI, OpenRouter, DeepSeek, Gemini's compatible endpoint, or a local llama.cpp or Ollama. Include the version part of the address, and name a model — there is no sensible default across all of them.",
  },
};

export function AiSettings({ settings, onSaved }) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [said, setSaid] = useState(null);

  const provider = SERVICES[settings.llm_provider] ? settings.llm_provider : "anthropic";
  const service = SERVICES[provider];

  async function save(patch) {
    try {
      await api("/api/settings", { method: "PUT", body: patch });
      onSaved();
    } catch (e) {
      setSaid({ ok: false, text: e.message });
    }
  }

  async function saveKey() {
    setBusy(true);
    setSaid({ ok: true, text: "checking…" });
    try {
      const res = await api("/api/settings/llm-key", {
        method: "PUT",
        body: { api_key: key },
      });
      setKey("");
      setSaid({ ok: res.ok === true, text: res.detail || "saved" });
      onSaved();
    } catch (e) {
      setSaid({ ok: false, text: e.message });
    } finally {
      setBusy(false);
    }
  }

  const keyHint = settings.llm_has_key
    ? "A key is stored. Paste another to replace it, or save an empty box to remove it."
    : settings.llm_key_from_env
      ? "A key is set in KOTODEX_ANTHROPIC_API_KEY and is what answers. It cannot be changed here — paste one to store a key that takes over from it."
      : "Needed for explaining a line, and for the short gloss on a mined card. Everything else works without one.";

  const keyPlaceholder = settings.llm_has_key
    ? "a key is stored"
    : settings.llm_key_from_env
      ? "a key is set in the environment"
      : "paste a key";

  return html`
    <div class="settings-group">
      <h3>AI</h3>
      <div class="settings-row">
        <label for="set-llm-key">API key</label>
        <div class="settings-input">
          <input
            id="set-llm-key"
            type="password"
            spellcheck="false"
            autocomplete="off"
            placeholder=${keyPlaceholder}
            value=${key}
            onInput=${(e) => setKey(e.currentTarget.value)}
          />
          <button type="button" onClick=${saveKey} disabled=${busy}>
            ${busy ? "checking…" : "save"}
          </button>
        </div>
        <p class="settings-hint">
          ${said
            ? html`<span class=${said.ok ? "goal-met" : "settings-err"}>${said.text}</span>`
            : keyHint}
        </p>
      </div>

      <div class="settings-row">
        <label>Service</label>
        <div class="settings-input">
          <div class="radio-set" role="radiogroup" aria-label="AI service">
            ${Object.entries(SERVICES).map(
              ([id, s]) => html`
                <label class="radio-opt" key=${id}>
                  <input
                    type="radio"
                    name="llm-provider"
                    value=${id}
                    checked=${provider === id}
                    onChange=${() => save({ llm_provider: id })}
                  />
                  <span>${s.label}</span>
                </label>
              `,
            )}
          </div>
        </div>
        <p class="settings-hint">${service.note}</p>
      </div>

      <div class="settings-row">
        <label for="set-llm-base-url">Address</label>
        <div class="settings-input">
          <input
            id="set-llm-base-url"
            type="text"
            spellcheck="false"
            placeholder=${service.baseUrl}
            value=${settings.llm_base_url}
            onChange=${(e) => save({ llm_base_url: e.currentTarget.value.trim() })}
          />
        </div>
        <p class="settings-hint">
          Empty uses the service's own. Set this to reach a proxy, a gateway or a
          model running on this machine.
        </p>
      </div>

      <div class="settings-row">
        <label for="set-llm-model">Model</label>
        <div class="settings-input">
          <input
            id="set-llm-model"
            type="text"
            spellcheck="false"
            placeholder="leave empty for the default"
            value=${settings.llm_model}
            onChange=${(e) => save({ llm_model: e.currentTarget.value.trim() })}
          />
        </div>
        <p class="settings-hint">
          Empty leaves each prompt on the model it was tuned against — a stronger
          one writes the card gloss, a cheaper one explains a line. Naming one
          here uses it for both.
        </p>
      </div>
    </div>
  `;
}
