---
name: ai-gate-axis-coverage
description: Adding a DeckFeatures axis + policy requires a floor-crossing gate matchup, or the ai-gate ships the policy dormant
metadata:
  type: feedback
---

When adding a new `DeckFeatures` axis + `TacticalPolicy` to `phase-ai`, the `cargo ai-gate` paired-seed report is only meaningful if a matchup's deck actually clears the policy's `COMMITMENT_FLOOR`. `activation()` returns `None` below the floor, so a gate run over decks that don't cross it exercises the policy *zero times* — a green "0 FAIL" then proves non-interference, not non-regression of the new behavior (this exact gap shipped the ArtifactSynergyPolicy un-exercised; the attached gate was red-mirror only).

**Why:** the merge gate is supposed to guard the policy's deltas; if the policy never fires during the gate, the gate is vacuous for that code.

**How to apply:**
- `crates/phase-ai/src/duel_suite/mod.rs` `FeatureKind` must get one variant per `DeckFeatures` field (enforced by `feature_kind_matches_deck_features_field_count` in `duel_suite/tests.rs`, which hardcodes the count — bump it).
- Tag a real floor-crossing matchup in `spec.rs::MATCHUPS` `exercises` (`every_feature_kind_is_exercised` enforces ≥1). Verify the deck crosses the floor with a `#[ignore]` DB-backed test (see `affinity_mirror_deck_activates_artifact_synergy`) — don't assume.
- The committed gate baseline (`crates/phase-ai/baselines/suite-baseline.json`) currently holds only `red-mirror`; the gate only runs/compares matchups present there, so a new axis also needs a full-suite baseline refresh to be covered.
- Don't try to confirm a small-nudge policy fired via suite attribution: `search.rs` `emit_trace_for_candidate` truncates to the top-3 policies by `|delta|`, and `attribution.rs` keeps top-3, so a 0.2–0.5 nudge is invisible behind larger penalties. Prove activation with a deterministic commitment-floor test instead.

Related: [[ai-policy-band-helpers]]
