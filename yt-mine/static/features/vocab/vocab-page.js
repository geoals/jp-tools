import { html } from 'htm/preact';
import { useState, useMemo } from 'preact/hooks';
import { tokenizeVocab, submitVocab } from '../../api.js';

const STATUS_OPTIONS = ['seen', 'known', 'blacklisted'];
const STATUS_LABELS = { seen: 'seen', known: 'known', blacklisted: 'blacklist' };

export function VocabPage() {
  const [text, setText] = useState('');
  const [tokens, setTokens] = useState(null);
  const [statuses, setStatuses] = useState({});
  const [sortBy, setSortBy] = useState('occurrence');
  const [filterBy, setFilterBy] = useState('all');
  const [posFilter, setPosFilter] = useState(null);
  const [tokenizing, setTokenizing] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [result, setResult] = useState(null);
  const [error, setError] = useState(null);

  async function handleTokenize(e) {
    e.preventDefault();
    setError(null);
    setResult(null);
    setTokenizing(true);
    try {
      const data = await tokenizeVocab(text);
      setTokens(data.tokens);
      const initial = {};
      for (const t of data.tokens) {
        const key = `${t.lemma}\t${t.reading}`;
        initial[key] = t.status || (t.in_dictionary ? 'seen' : 'blacklisted');
      }
      setStatuses(initial);
      setPosFilter(new Set(data.tokens.map((t) => t.pos)));
    } catch (err) {
      setError(err.message);
    } finally {
      setTokenizing(false);
    }
  }

  function setStatus(lemma, reading, status) {
    const key = `${lemma}\t${reading}`;
    setStatuses((prev) => ({ ...prev, [key]: status }));
  }

  async function handleSubmit() {
    setError(null);
    setResult(null);
    setSubmitting(true);
    try {
      const entries = tokens.map((t) => ({
        lemma: t.lemma,
        reading: t.reading,
        pos: t.pos,
        status: statuses[`${t.lemma}\t${t.reading}`] || 'seen',
        count: t.count,
      }));
      const data = await submitVocab(entries);
      setResult(`Submitted ${data.count} entries.`);
      setTokens(null);
    } catch (err) {
      setError(err.message);
    } finally {
      setSubmitting(false);
    }
  }

  const allPos = useMemo(() => {
    if (!tokens) return [];
    const seen = new Set();
    return tokens.map((t) => t.pos).filter((p) => seen.has(p) ? false : (seen.add(p), true));
  }, [tokens]);

  function togglePos(pos) {
    setPosFilter((prev) => {
      const next = new Set(prev);
      next.has(pos) ? next.delete(pos) : next.add(pos);
      return next;
    });
  }

  function toggleAllPos() {
    setPosFilter((prev) =>
      prev && prev.size === allPos.length ? new Set() : new Set(allPos)
    );
  }

  const allPosSelected = posFilter && posFilter.size === allPos.length;

  const filteredTokens = tokens && tokens.filter((t) => {
    if (posFilter && !posFilter.has(t.pos)) return false;
    if (filterBy === 'in-dict') return t.in_dictionary;
    if (filterBy === 'not-in-dict') return !t.in_dictionary;
    return true;
  });

  const sortedTokens = filteredTokens && [...filteredTokens].sort((a, b) =>
    sortBy === 'count'
      ? b.count - a.count || a.first_occurrence - b.first_occurrence
      : a.first_occurrence - b.first_occurrence
  );

  return html`
    <div class="vocab-page">
      <h2>Vocabulary Calibration</h2>
      <p class="helper-text">Paste Japanese text to review and categorize words.</p>

      <form onSubmit=${handleTokenize}>
        <textarea
          class="vocab-textarea"
          placeholder="日本語のテキストをここに貼り付けてください..."
          value=${text}
          onInput=${(e) => setText(e.target.value)}
          rows="6"
        />
        <button type="submit" disabled=${tokenizing || !text.trim()}>
          ${tokenizing && html`<span class="spinner"></span>`}
          <span>Tokenize</span>
        </button>
      </form>

      ${error && html`<p class="vocab-message vocab-error">${error}</p>`}
      ${result && html`<p class="vocab-message vocab-success">${result}</p>`}

      ${sortedTokens && html`
        <div class="vocab-sort-bar">
          <span>Sort by:</span>
          <button
            class="sort-toggle ${sortBy === 'occurrence' ? 'active' : ''}"
            onClick=${() => setSortBy('occurrence')}
          >Occurrence</button>
          <button
            class="sort-toggle ${sortBy === 'count' ? 'active' : ''}"
            onClick=${() => setSortBy('count')}
          >Count</button>
        </div>

        <div class="vocab-filter-bar">
          <span>Filter:</span>
          <button
            class="filter-toggle ${filterBy === 'all' ? 'active' : ''}"
            onClick=${() => setFilterBy('all')}
          >All</button>
          <button
            class="filter-toggle ${filterBy === 'in-dict' ? 'active' : ''}"
            onClick=${() => setFilterBy('in-dict')}
          >In Dict</button>
          <button
            class="filter-toggle ${filterBy === 'not-in-dict' ? 'active' : ''}"
            onClick=${() => setFilterBy('not-in-dict')}
          >Not in Dict</button>
        </div>

        <div class="vocab-filter-bar">
          <span>Part-of-speech:</span>
          <span class="vocab-pos-buttons">
            <button
              class="filter-toggle-all ${allPosSelected ? 'active' : ''}"
              onClick=${toggleAllPos}
            >All</button>
            ${allPos.map((pos) => html`
              <button
                class="filter-toggle ${posFilter && posFilter.has(pos) ? 'active' : ''}"
                onClick=${() => togglePos(pos)}
              >${pos}</button>
            `)}
          </span>
        </div>

        <table class="vocab-table">
          <thead>
            <tr>
              <th>Word</th>
              <th>Reading</th>
              <th>POS</th>
              <th>Dict</th>
              <th>Count</th>
              <th>Status</th>
            </tr>
          </thead>
          <tbody>
            ${sortedTokens.map((t) => {
              const key = `${t.lemma}\t${t.reading}`;
              const current = statuses[key] || 'seen';
              return html`
                <tr class="${t.in_db ? 'in-db' : ''} ${!t.in_dictionary ? 'not-in-dict' : ''}">
                  <td class="vocab-lemma">${t.lemma}</td>
                  <td class="vocab-reading">${t.reading}</td>
                  <td class="vocab-pos">${t.pos}</td>
                  <td class="vocab-dict">${t.in_dictionary ? '\u2713' : '\u2014'}</td>
                  <td class="vocab-count">${t.count}</td>
                  <td class="vocab-status-cell">
                    ${STATUS_OPTIONS.map((s) => html`
                      <button
                        type="button"
                        class="status-toggle ${s} ${current === s ? 'active' : ''}"
                        onClick=${() => setStatus(t.lemma, t.reading, s)}
                      >${STATUS_LABELS[s]}</button>
                    `)}
                  </td>
                </tr>
              `;
            })}
          </tbody>
        </table>

        <button
          class="vocab-submit"
          onClick=${handleSubmit}
          disabled=${submitting}
        >
          ${submitting && html`<span class="spinner"></span>`}
          <span>Submit ${tokens.length} words</span>
        </button>
      `}
    </div>
  `;
}
