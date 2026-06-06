//! CR 608.2c: Interactive battlefield-object selection into the chain's
//! tracked object set.
//!
//! Resolves `Effect::ChooseObjectsIntoTrackedSet`. The `chooser` field is a
//! `TargetFilter` resolved per-instance (like `Effect::PayCost.payer`) so an
//! "at the beginning of each player's upkeep" trigger prompts the player whose
//! upkeep it is — not a fixed controller. The chosen objects are written into
//! a fresh tracked set so downstream effects ("pay {N} for each ... chosen
//! this way", "untap those creatures") resolve against the exact selection.

use crate::game::filter::{matches_target_filter, FilterContext};
use crate::game::targeting::resolve_effect_player_ref;
use crate::types::ability::{Effect, EffectError, EffectKind, ResolvedAbility, TargetRef};
use crate::types::events::GameEvent;
use crate::types::game_state::{GameState, WaitingFor};

/// CR 608.2c: Resolve `Effect::ChooseObjectsIntoTrackedSet` — surface a
/// `WaitingFor::ChooseObjectsSelection` prompt for the affected player.
pub fn resolve(
    state: &mut GameState,
    ability: &ResolvedAbility,
    events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    let (chooser_filter, filter) = match &ability.effect {
        Effect::ChooseObjectsIntoTrackedSet {
            chooser, filter, ..
        } => (chooser.clone(), filter.clone()),
        _ => {
            return Err(EffectError::MissingParam(
                "ChooseObjectsIntoTrackedSet".to_string(),
            ))
        }
    };

    // CR 608.2c: Resolve the chooser to the affected player — the same
    // single-authority player-ref resolver used by `PayCost.payer`. For an
    // "each player's upkeep" trigger this is the upkeep player.
    let Some(chooser) = resolve_effect_player_ref(state, ability, &chooser_filter) else {
        // No resolvable chooser — nothing to select; resolve as a no-op.
        events.push(GameEvent::EffectResolved {
            kind: EffectKind::from(&ability.effect),
            source_id: ability.source_id,
        });
        return Ok(());
    };

    // Evaluate `filter` against the eligible objects. The filter's "they
    // control" controller constraint resolves against the ability controller,
    // so bind the filter context controller to the chooser (mirrors `pay.rs`'s
    // payer-rebinding pattern).
    let ctx = FilterContext::from_ability_with_controller(ability, chooser);
    // CR 608.2c: scan the zone(s) the filter explicitly names; default to the
    // battlefield. A hand-zone filter (Amplify's "reveal any number of cards
    // from your hand that share a creature type with it", CR 702.38a) selects
    // from the chooser's hand instead of the battlefield.
    let zones = crate::game::targeting::extract_explicit_zones(&filter);
    let candidate_ids: Vec<crate::types::identifiers::ObjectId> = if zones
        .iter()
        .any(|z| *z != crate::types::zones::Zone::Battlefield)
    {
        zones
            .iter()
            .flat_map(|zone| crate::game::targeting::zone_object_ids(state, *zone))
            .collect()
    } else {
        state.battlefield.iter().copied().collect()
    };
    let eligible: Vec<TargetRef> = candidate_ids
        .into_iter()
        .filter(|&obj_id| matches_target_filter(state, obj_id, &filter, &ctx))
        .map(TargetRef::Object)
        .collect();

    // CR 608.2c: Surface the interactive selection. Even with an empty
    // `eligible` set the prompt is raised — the player's act of submitting an
    // empty selection IS the acknowledgment (CR 118.5), and the downstream
    // `ScaledMana { times: 0 }` payment is a no-op SUCCESS.
    // CR 608.2: carry the triggering event across the interactive selection
    // pause so the stashed `PayCost { payer: TriggeringPlayer }` continuation
    // resolves the payer correctly. PART 1 has already restored
    // `current_trigger_event`, so this clone captures the real event.
    state.waiting_for = WaitingFor::ChooseObjectsSelection {
        player: chooser,
        eligible,
        trigger_event: state.current_trigger_event.clone(),
    };

    Ok(())
}
