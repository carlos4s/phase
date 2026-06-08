//! Tests for Epic (CR 702.50). Declared from `effects/mod.rs` so `epic.rs`
//! stays implementation-only.

use super::epic::{arm_epic, is_epic_locked, resolve};
use crate::game::zones::create_object;
use crate::types::ability::{
    DelayedTriggerCondition, Effect, QuantityExpr, ResolvedAbility, TargetFilter,
};
use crate::types::events::GameEvent;
use crate::types::game_state::{GameState, StackEntryKind};
use crate::types::identifiers::CardId;
use crate::types::keywords::Keyword;
use crate::types::phase::Phase;
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

fn gain_two_life() -> Effect {
    Effect::GainLife {
        amount: QuantityExpr::Fixed { value: 2 },
        player: TargetFilter::Controller,
    }
}

/// Create the resolved Epic spell's snapshot ability sourced to `src`.
fn snapshot(src: crate::types::identifiers::ObjectId) -> ResolvedAbility {
    ResolvedAbility::new(gain_two_life(), Vec::new(), src, PlayerId(0))
}

#[test]
fn arm_epic_locks_controller_and_arms_recurring_upkeep_trigger() {
    // CR 702.50a-b: arming locks the controller and installs a recurring
    // upkeep copy trigger.
    let mut state = GameState::new_two_player(42);
    let src = create_object(
        &mut state,
        CardId(1),
        PlayerId(0),
        "Enduring Ideal".to_string(),
        Zone::Graveyard,
    );

    assert!(!is_epic_locked(&state, PlayerId(0)));
    arm_epic(&mut state, src, PlayerId(0), snapshot(src));

    // CR 702.50b: the controller can no longer cast spells.
    assert!(is_epic_locked(&state, PlayerId(0)));
    assert!(!is_epic_locked(&state, PlayerId(1)));

    // CR 702.50a: exactly one recurring upkeep delayed trigger.
    assert_eq!(state.delayed_triggers.len(), 1);
    let trig = &state.delayed_triggers[0];
    assert!(!trig.one_shot, "Epic's copy trigger recurs every upkeep");
    assert_eq!(trig.controller, PlayerId(0));
    assert_eq!(trig.source_id, src);
    match &trig.condition {
        DelayedTriggerCondition::AtNextPhaseForPlayer { phase, player } => {
            assert_eq!(*phase, Phase::Upkeep);
            assert_eq!(*player, PlayerId(0));
        }
        other => panic!("expected AtNextPhaseForPlayer Upkeep, got {other:?}"),
    }
    assert!(matches!(trig.ability.effect, Effect::EpicCopy { .. }));
}

#[test]
fn each_epic_resolution_arms_an_independent_trigger() {
    // CR 702.50a: two Epic spells → two independent recurring triggers.
    let mut state = GameState::new_two_player(42);
    let a = create_object(
        &mut state,
        CardId(1),
        PlayerId(0),
        "A".into(),
        Zone::Graveyard,
    );
    let b = create_object(
        &mut state,
        CardId(2),
        PlayerId(0),
        "B".into(),
        Zone::Graveyard,
    );
    arm_epic(&mut state, a, PlayerId(0), snapshot(a));
    arm_epic(&mut state, b, PlayerId(0), snapshot(b));
    assert_eq!(state.delayed_triggers.len(), 2);
}

#[test]
fn epic_copy_puts_a_keyword_stripped_copy_on_the_stack() {
    // CR 702.50a + CR 707.10: resolving EpicCopy puts a copy of the spell on the
    // stack, excluding the epic ability so it does not recurse.
    let mut state = GameState::new_two_player(42);
    let proto = create_object(
        &mut state,
        CardId(7),
        PlayerId(0),
        "Enduring Ideal".to_string(),
        Zone::Graveyard,
    );
    // The graveyard Epic card still carries the keyword.
    state
        .objects
        .get_mut(&proto)
        .unwrap()
        .keywords
        .push(Keyword::Epic);

    let epic_ability = ResolvedAbility::new(
        Effect::EpicCopy {
            spell: Box::new(snapshot(proto)),
        },
        Vec::new(),
        proto,
        PlayerId(0),
    );

    let mut events = Vec::new();
    resolve(&mut state, &epic_ability, &mut events).expect("EpicCopy resolves");

    // A copy was put on the stack.
    let top = state.stack.back().expect("a copy is on the stack");
    let copy_id = top.id;
    assert!(matches!(top.kind, StackEntryKind::Spell { .. }));

    // CR 707.10: the copy is a token spell; CR 702.50a: Epic is stripped so the
    // copy's own resolution won't arm a second Epic effect.
    let copy = state.objects.get(&copy_id).expect("copy object exists");
    assert!(copy.is_token);
    assert!(
        !copy.keywords.iter().any(|k| matches!(k, Keyword::Epic)),
        "the copy must exclude the epic ability"
    );

    // CR 707.10: a copy is placed, not cast — SpellCopied, not SpellCast.
    assert!(events
        .iter()
        .any(|e| matches!(e, GameEvent::SpellCopied { .. })));
    assert!(!events
        .iter()
        .any(|e| matches!(e, GameEvent::SpellCast { .. })));
}

#[test]
fn epic_copy_is_a_noop_when_the_prototype_is_gone() {
    // CR 608.2: with no last-known prototype object, no copy can be built.
    let mut state = GameState::new_two_player(42);
    let missing = crate::types::identifiers::ObjectId(9999);
    let epic_ability = ResolvedAbility::new(
        Effect::EpicCopy {
            spell: Box::new(snapshot(missing)),
        },
        Vec::new(),
        missing,
        PlayerId(0),
    );

    let mut events = Vec::new();
    resolve(&mut state, &epic_ability, &mut events).expect("resolves as a no-op");
    assert!(state.stack.is_empty(), "no copy is created");
}
