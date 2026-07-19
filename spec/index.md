# jp-tools — Japanese Language Learning Toolkit

> **Historical.** These documents are the original pre-implementation design
> (single unified HTTP API, LinDera/MeCab tokenizer, calibration UI, a
> `vocabulary` knowledge-state DB). What actually got built is different: four
> independent services (`yt-mine`, `manga-mine`, `vn-mine`, `read-stats`)
> sharing the `jp-core`/`jp-mine-core` library crates, tokenizing with Sudachi.
> For current architecture and status, see the root [`CLAUDE.md`](../CLAUDE.md)
> and each project's own `CLAUDE.md`/`README.md`. Kept here for the reasoning
> behind early design decisions, not as a description of the current system.

A personal toolkit for intermediate+ Japanese learners who consume native media
(visual novels, YouTube, books) and want intelligent vocabulary tracking,
contextual word explanations, and streamlined Anki card mining.

## Core Problem

Existing tools (Migaku, jpdb.io, Language Reactor, etc.) assume you either start
from scratch or already have a complete knowledge profile. For an intermediate
learner with ~1500 Anki cards but significantly larger passive vocabulary, there
is no good way to bootstrap a knowledge database that reflects actual ability.
Without that database, features like unknown-word highlighting and i+1 sentence
filtering are useless.

## Design Principles

- **Solve the cold-start problem first.** Nothing else works without an accurate
  knowledge base.
- **Complement existing tools, don't replace them.** Yomitan stays as the popup
  dictionary. Anki stays as the SRS. This tool is the knowledge-state layer and
  mining pipeline that connects them.
- **LLMs for insight, morphological analysis for structure.** Tokenization is
  deterministic and cheap — use MeCab/LinDera/Vibrato. LLMs provide the
  nuanced, contextual word explanations that dictionaries can't.
- **Single-user, local-first.** SQLite, no server infrastructure. Data stays on
  your machine.

## Spec Documents

- [Data Model](./data-model.md) — Database schema and key design decisions
- [Cold Start](./cold-start.md) — Bootstrapping the knowledge base
- [Features](./features.md) — Feature specs, priorities, and feasibility
- [Yomitan Integration](./yomitan-integration.md) — Options for working with Yomitan
- [Architecture](./architecture.md) — Technical stack, component overview, morphological analysis + LLM pipeline
- [Open Questions](./open-questions.md) — Unresolved decisions and things to research
- [Sentence Mining](./sentence-mining-yt.md) — YouTube sentence mining pipeline, MVP phases, and architecture
- [Sentence Mining — Manga](./sentence-mining-manga.md) — physical manga mining: photo → crop → OCR → rich Anki card, sharing yt-mine's core

## Status

**Superseded.** This describes the pre-implementation plan, not the current
system — see the historical note at the top of this document.
