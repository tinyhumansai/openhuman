# Memory tree — score signals

Per-chunk scoring features. Each submodule computes one signal in `[0.0, 1.0]`; `ops::combine` aggregates them via `SignalWeights` into the final admission total. Signals are stored alongside the total in `mem_tree_score` so admit/drop decisions remain auditable.

## Files

- `mod.rs` — module surface: re-exports `compute`, `combine`, `combine_cheap_only`, `entity_density_score`, `ScoreSignals`, `SignalWeights`.
- `types.rs` — `ScoreSignals` (per-signal breakdown) and `SignalWeights` (per-signal multipliers, with `with_llm_enabled()` builder).
- `ops.rs` — `compute(meta, content, token_count, extracted)` populates a `ScoreSignals`; `combine` and `combine_cheap_only` produce the weighted total (the latter excludes the LLM-importance term used by the borderline-band short-circuit).
- `token_count.rs` — plateau-shaped score over chunk token count; scores 0 below `TOKEN_MIN`, ramps to 1 by `TOKEN_RAMP_LOW`, ramps back to 0.5 between `TOKEN_RAMP_HIGH` and `TOKEN_MAX`.
- `unique_words.rs` — type-token-ratio noise detector: low diversity scores low; messages under `MIN_TOTAL_WORDS` return a neutral 0.5.
- `metadata_weight.rs` — base weight per `SourceKind` (Email > Document > Chat).
- `source_weight.rs` — per-`DataSource` weight inferred from `provider:<name>` tags, with `SourceKind` defaults as fallback.
- `interaction.rs` — engagement-tag bonus (`sent`, `reply`, `dm`, `mention`); absent tags return 0.5 so silent content isn't penalised.
