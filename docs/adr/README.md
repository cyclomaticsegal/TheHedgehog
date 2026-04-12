# Architecture Decision Records

This directory contains Architecture Decision Records (ADRs) for the Regime Shift project.

## What is an ADR?

An ADR captures a significant technical decision — what was decided, why, and what tradeoffs were accepted. ADRs are immutable once accepted; superseded decisions get a new ADR that references the old one.

## Naming convention

```
NNNN-short-kebab-case-title.md
```

`NNNN` is a zero-padded four-digit sequence number. Numbers are never reused.

## Record structure

Each ADR contains:

| Section | Purpose |
|---------|---------|
| **Status** | `Draft` → `Accepted` → `Superseded by ADR-XXXX` (or `Rejected`) |
| **Date** | Date accepted (YYYY-MM-DD) |
| **Context** | The problem, constraints, and forces at play |
| **Decision** | What was decided and the rationale |
| **Consequences** | Positive outcomes, neutral trade-offs, and known downsides |

## Index

| # | Title | Status | Date |
|---|-------|--------|------|
| [0001](0001-performance-improvements.md) | Performance improvements — memory allocation and hot-path optimisations | Accepted | 2026-04-02 |
| [0002](0002-price-panel-ux.md) | Price panel — keyboard-triggered drill-down from correlation chart | Accepted | 2026-04-02 |
| [0003](0003-collapsible-chart-panels.md) | Collapsible chart panels with summary headers | Accepted | 2026-04-02 |
| [0004](0004-design-system-colour-palette.md) | Design system — colour palette and global dark theme | Accepted | 2026-04-02 |
| [0005](0005-rag-ai-analysis-panel.md) | RAG-Powered AI Analysis Panel | Accepted | 2026-04-03 |
| [0006](0006-inference-persistence-and-reports.md) | Inference Persistence and Summary Reports | Accepted | 2026-04-04 |
| [0007](0007-rollback-tabs-dexter-focus-51folds.md) | Roll Back Tabbed Workspace; Focus Integration on 51Folds | Accepted | 2026-04-05 |
| [0008](0008-51folds-integration.md) | 51Folds Integration — Hypothesis Generation and Bayesian Model Creation | Accepted | 2026-04-06 |
| [0009](0009-strengthen-hypothesis-quality-and-fix-openai-tool-compatibility.md) | Strengthen Hypothesis Quality and Fix OpenAI Tool Compatibility | Accepted | 2026-04-06 |
| [0010](0010-multi-provider-commodity-caching.md) | Multi-Provider Commodity Data Caching | Superseded by 0011 | 2026-04-07 |
| [0011](0011-single-provider-daily-cache.md) | Single-Provider Commodity Pipeline with Daily Cache | Accepted | 2026-04-07 |
| [0012](0012-analysis-quality-folds-persistence-and-editor-consolidation.md) | Analysis Quality Hardening, Persistent 51Folds Tracking, and AI Editor Consolidation | Accepted | 2026-04-07 |
| [0013](0013-sdk-integration-model-explorer-ui.md) | 51Folds Rust SDK Integration, Rich Model Explorer, and Tabbed Central Panel | Accepted | 2026-04-10 |
| [0014](0014-model-explorer-navigation-stack-ui-redesign.md) | Model Explorer Navigation Stack UI Redesign | Accepted | 2026-04-10 |
| [0015](0015-dark-theme-hardening-and-51folds-ui-polish.md) | Dark Theme Hardening and 51Folds Model Explorer UI Polish | Accepted | 2026-04-11 |
| [0016](0016-splash-screen-revert-architecture-and-51folds-patch-drift.md) | Splash Screen, Revert Architecture, and 51Folds PATCH/PUT Drift | In limbo | 2026-04-11 |
