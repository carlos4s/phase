//! Unstable Contraption deck, assemble, and crank runtime.

use crate::game::effects::choose_one_of;
use crate::game::filter::{matches_target_filter, FilterContext};
use crate::game::game_object::GameObject;
use crate::game::quantity::resolve_quantity_with_targets;
use crate::game::targeting::resolved_object_ids_for_filter;
use crate::types::ability::{
    AbilityDefinition, AbilityKind, Effect, EffectError, EffectKind, ResolvedAbility,
    SubAbilityLink, TargetFilter,
};
use crate::types::events::GameEvent;
use crate::types::game_state::{BatchCompletion, GameState, PendingContinuation, WaitingFor};
use crate::types::identifiers::{ObjectId, TrackedSetId};
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

pub fn is_contraption_card(obj: &GameObject) -> bool {
    obj.in_contraption_deck
        || obj
            .card_types
            .subtypes
            .iter()
            .any(|subtype| subtype.eq_ignore_ascii_case("Contraption"))
}

pub fn is_contraption_permanent(obj: &GameObject) -> bool {
    obj.zone == Zone::Battlefield && is_contraption_card(obj)
}

pub fn resolve(
    state: &mut GameState,
    ability: &ResolvedAbility,
    events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    match &ability.effect {
        Effect::AssembleContraptions { count } => {
            let count = resolve_quantity_with_targets(state, count, ability).max(0) as u32;
            prompt_assemble_sprocket_choice(state, ability, count);
            events.push(GameEvent::EffectResolved {
                kind: EffectKind::AssembleContraptions,
                source_id: ability.source_id,
            });
            Ok(())
        }
        Effect::AssembleContraptionsFromRollDifference => {
            let count = recent_roll_difference(events);
            prompt_assemble_sprocket_choice(state, ability, count);
            events.push(GameEvent::EffectResolved {
                kind: EffectKind::AssembleContraptionsFromRollDifference,
                source_id: ability.source_id,
            });
            Ok(())
        }
        Effect::AssembleContraptionOnSprocket {
            sprocket,
            remaining,
        } => {
            assemble_one_onto_sprocket(
                state,
                ability.controller,
                ability.source_id,
                *sprocket,
                *remaining,
                events,
            )?;
            events.push(GameEvent::EffectResolved {
                kind: EffectKind::AssembleContraptionOnSprocket,
                source_id: ability.source_id,
            });
            Ok(())
        }
        Effect::CrankContraptions { target } => {
            crank_selected_contraptions(state, ability, target, events);
            events.push(GameEvent::EffectResolved {
                kind: EffectKind::CrankContraptions,
                source_id: ability.source_id,
            });
            Ok(())
        }
        Effect::ReassembleContraption {
            target,
            gain_control,
        } => {
            prompt_reassemble_sprocket_choice(state, ability, target, *gain_control)?;
            events.push(GameEvent::EffectResolved {
                kind: EffectKind::ReassembleContraption,
                source_id: ability.source_id,
            });
            Ok(())
        }
        Effect::ReassembleContraptionOnSprocket {
            target,
            sprocket,
            gain_control,
        } => {
            apply_reassemble_to_sprocket(state, ability, target, *sprocket, *gain_control, events)?;
            events.push(GameEvent::EffectResolved {
                kind: EffectKind::ReassembleContraptionOnSprocket,
                source_id: ability.source_id,
            });
            Ok(())
        }
        _ => Err(EffectError::MissingParam("Contraptions".to_string())),
    }
}

pub fn perform_contraption_upkeep_turn_based_action(
    state: &mut GameState,
    events: &mut Vec<GameEvent>,
) -> Option<WaitingFor> {
    let player = state.active_player;
    if !controls_contraption(state, player) {
        return None;
    }

    let sprocket = advance_crank_sprocket(state, player);
    let eligible: Vec<_> = state
        .battlefield
        .iter()
        .copied()
        .filter(|id| {
            state.objects.get(id).is_some_and(|obj| {
                obj.controller == player
                    && obj.is_phased_in()
                    && is_contraption_permanent(obj)
                    && obj.contraption_sprocket == Some(sprocket)
            })
        })
        .map(crate::types::ability::TargetRef::Object)
        .collect();

    if eligible.is_empty() {
        return None;
    }

    let continuation = ResolvedAbility::new(
        Effect::CrankContraptions {
            target: TargetFilter::TrackedSet {
                id: TrackedSetId(0),
            },
        },
        Vec::new(),
        ObjectId(0),
        player,
    );
    state.pending_continuation = Some(PendingContinuation::new(Box::new(continuation)));
    state.waiting_for = WaitingFor::ChooseObjectsSelection {
        player,
        eligible,
        trigger_event: None,
    };
    let _ = events;
    Some(state.waiting_for.clone())
}

pub(crate) fn finish_contraption_assembly(
    state: &mut GameState,
    player: PlayerId,
    object_id: ObjectId,
    sprocket: u8,
    events: &mut Vec<GameEvent>,
) {
    if let Some(obj) = state.objects.get_mut(&object_id) {
        obj.in_contraption_deck = false;
        obj.contraption_sprocket = (obj.zone == Zone::Battlefield).then_some(sprocket);
    }
    if state
        .objects
        .get(&object_id)
        .is_some_and(|obj| obj.zone == Zone::Battlefield)
    {
        events.push(GameEvent::ContraptionAssembled {
            player_id: player,
            object_id,
            sprocket,
        });
    }
}

fn prompt_assemble_sprocket_choice(state: &mut GameState, ability: &ResolvedAbility, count: u32) {
    let count = apply_assemble_replacements(state, ability.source_id, count);
    let available = state
        .players
        .iter()
        .find(|player| player.id == ability.controller)
        .map(|player| player.contraption_deck.len() as u32)
        .unwrap_or(0);
    let count = count.min(available);
    if count == 0 {
        return;
    }

    choose_one_of::prompt_next(
        state,
        ability.controller,
        ability.source_id,
        assemble_sprocket_branches(count),
        ability.targets.clone(),
        ability.context.clone(),
        vec![ability.controller],
    );
}

fn assemble_sprocket_branches(count: u32) -> Vec<AbilityDefinition> {
    [1_u8, 2, 3]
        .into_iter()
        .map(|sprocket| {
            let mut branch = AbilityDefinition::new(
                AbilityKind::Spell,
                Effect::AssembleContraptionOnSprocket {
                    sprocket,
                    remaining: count.saturating_sub(1),
                },
            )
            .description(format!("Put it onto sprocket {sprocket}."));
            branch.sub_link = SubAbilityLink::SequentialSibling;
            branch
        })
        .collect()
}

fn assemble_one_onto_sprocket(
    state: &mut GameState,
    player: PlayerId,
    source_id: ObjectId,
    sprocket: u8,
    remaining: u32,
    events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    let Some(object_id) = state
        .players
        .iter_mut()
        .find(|candidate| candidate.id == player)
        .and_then(|candidate| candidate.contraption_deck.pop_front())
    else {
        return Ok(());
    };

    match super::zone_pipeline::move_object(
        state,
        super::zone_pipeline::ZoneMoveRequest::effect(object_id, Zone::Battlefield, source_id),
        events,
    ) {
        super::zone_pipeline::ZoneMoveResult::Done => {
            finish_contraption_assembly(state, player, object_id, sprocket, events);
            if remaining > 0 {
                let synthetic = ResolvedAbility::new(
                    Effect::AssembleContraptions {
                        count: crate::types::ability::QuantityExpr::Fixed {
                            value: remaining as i32,
                        },
                    },
                    Vec::new(),
                    source_id,
                    player,
                );
                prompt_assemble_sprocket_choice(state, &synthetic, remaining);
            }
        }
        super::zone_pipeline::ZoneMoveResult::NeedsChoice(_)
        | super::zone_pipeline::ZoneMoveResult::NeedsAuraAttachmentChoice => {
            super::zone_pipeline::defer_completion_on_pause(
                state,
                BatchCompletion::ContraptionAssembleRemainder {
                    player,
                    source_id,
                    object_id,
                    sprocket,
                    remaining,
                },
            );
        }
    }
    Ok(())
}

fn crank_selected_contraptions(
    state: &mut GameState,
    ability: &ResolvedAbility,
    target: &TargetFilter,
    events: &mut Vec<GameEvent>,
) {
    let mut contraptions = resolved_object_ids_for_filter(state, ability, target);
    contraptions.sort_by_key(|id| id.0);
    contraptions.dedup();

    for contraption_id in contraptions {
        let Some(obj) = state.objects.get(&contraption_id) else {
            continue;
        };
        if obj.zone != Zone::Battlefield || !obj.is_phased_in() || !is_contraption_permanent(obj) {
            continue;
        }
        let Some(sprocket) = obj.contraption_sprocket else {
            continue;
        };
        events.push(GameEvent::ContraptionCranked {
            player_id: obj.controller,
            sprocket,
            contraption_id,
        });
    }

    super::triggers::process_triggers(state, events);
}

fn prompt_reassemble_sprocket_choice(
    state: &mut GameState,
    ability: &ResolvedAbility,
    target: &TargetFilter,
    gain_control: bool,
) -> Result<(), EffectError> {
    let mut targets = resolved_object_ids_for_filter(state, ability, target);
    targets.sort_by_key(|id| id.0);
    targets.dedup();
    let Some(target_id) = targets.first().copied() else {
        return Ok(());
    };
    let current_sprocket = state
        .objects
        .get(&target_id)
        .and_then(|obj| obj.contraption_sprocket);
    let branches: Vec<_> = [1_u8, 2, 3]
        .into_iter()
        .filter(|sprocket| Some(*sprocket) != current_sprocket)
        .map(|sprocket| {
            AbilityDefinition::new(
                AbilityKind::Spell,
                Effect::ReassembleContraptionOnSprocket {
                    target: TargetFilter::TrackedSet {
                        id: TrackedSetId(0),
                    },
                    sprocket,
                    gain_control,
                },
            )
            .description(format!("Move it onto sprocket {sprocket}."))
        })
        .collect();
    if branches.is_empty() {
        return Ok(());
    }

    state
        .tracked_object_sets
        .insert(TrackedSetId(0), vec![target_id]);
    state.chain_tracked_set_id = Some(TrackedSetId(0));
    choose_one_of::prompt_next(
        state,
        ability.controller,
        ability.source_id,
        branches,
        ability.targets.clone(),
        ability.context.clone(),
        vec![ability.controller],
    );
    Ok(())
}

fn apply_reassemble_to_sprocket(
    state: &mut GameState,
    ability: &ResolvedAbility,
    target: &TargetFilter,
    sprocket: u8,
    gain_control: bool,
    _events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    let mut targets = resolved_object_ids_for_filter(state, ability, target);
    targets.sort_by_key(|id| id.0);
    targets.dedup();
    let Some(target_id) = targets.first().copied() else {
        return Ok(());
    };
    let Some(obj) = state.objects.get_mut(&target_id) else {
        return Ok(());
    };
    if obj.zone != Zone::Battlefield || !is_contraption_card(obj) {
        return Ok(());
    }
    if gain_control {
        obj.controller = ability.controller;
        obj.base_controller = Some(ability.controller);
    }
    obj.contraption_sprocket = Some(sprocket);
    Ok(())
}

fn controls_contraption(state: &GameState, player: PlayerId) -> bool {
    state.battlefield.iter().any(|id| {
        state.objects.get(id).is_some_and(|obj| {
            obj.controller == player && obj.is_phased_in() && is_contraption_permanent(obj)
        })
    })
}

fn advance_crank_sprocket(state: &mut GameState, player: PlayerId) -> u8 {
    let player_state = state
        .players
        .iter_mut()
        .find(|candidate| candidate.id == player)
        .expect("active player exists");
    let next = next_sprocket(player_state.contraption_crank_sprocket);
    player_state.contraption_crank_sprocket = next;
    next
}

fn next_sprocket(current: u8) -> u8 {
    match current {
        1 => 2,
        2 => 3,
        _ => 1,
    }
}

fn recent_roll_difference(events: &[GameEvent]) -> u32 {
    let mut rolls = events.iter().rev().filter_map(|event| match event {
        GameEvent::DieRolled {
            result: Some(result),
            ..
        } => Some(*result),
        _ => None,
    });
    let Some(first) = rolls.next() else {
        return 0;
    };
    let Some(second) = rolls.next() else {
        return 0;
    };
    u8::abs_diff(first, second) as u32
}

fn apply_assemble_replacements(state: &GameState, source_id: ObjectId, count: u32) -> u32 {
    let mut adjusted = count;
    for (replacement_source_id, replacement_source) in &state.objects {
        if replacement_source.zone != Zone::Battlefield || !replacement_source.is_phased_in() {
            continue;
        }
        for replacement in replacement_source.replacement_definitions.iter_unchecked() {
            if replacement.event
                != crate::types::replacements::ReplacementEvent::AssembleContraption
            {
                continue;
            }
            let matches_source = replacement.valid_card.as_ref().is_none_or(|filter| {
                let ctx = FilterContext::from_source_with_controller(
                    *replacement_source_id,
                    replacement_source.controller,
                );
                matches_target_filter(state, source_id, filter, &ctx)
            });
            if matches_source {
                adjusted = adjusted.saturating_mul(2);
            }
        }
    }
    adjusted
}
