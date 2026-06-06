//! Tests for Amplify (CR 702.38a) synthesis and runtime. Declared from
//! `database/mod.rs` so the implementation module (`amplify.rs`) stays free of
//! inline test scaffolding.

use std::sync::Arc;

use super::amplify::synthesize_amplify;
use crate::database::mtgjson::{AtomicCard, AtomicIdentifiers};
use crate::game::engine::apply_as_current;
use crate::game::stack::resolve_top;
use crate::game::triggers::process_triggers;
use crate::game::zones::{create_object, move_to_zone};
use crate::types::ability::{
    Effect, FilterProp, QuantityExpr, QuantityRef, TargetFilter, TargetRef,
};
use crate::types::actions::GameAction;
use crate::types::card::CardFace;
use crate::types::card_type::CoreType;
use crate::types::counter::CounterType;
use crate::types::game_state::{GameState, StackEntryKind, WaitingFor};
use crate::types::identifiers::{CardId, ObjectId};
use crate::types::keywords::Keyword;
use crate::types::player::PlayerId;
use crate::types::triggers::TriggerMode;
use crate::types::zones::Zone;

// ---------------------------------------------------------------------------
// Synthesis-shape tests
// ---------------------------------------------------------------------------

fn face_with_amplify(n: u32) -> CardFace {
    let mut face = CardFace::default();
    face.card_type.core_types.push(CoreType::Creature);
    face.keywords.push(Keyword::Amplify(n));
    face
}

fn amplify_trigger(face: &CardFace) -> &crate::types::ability::TriggerDefinition {
    face.triggers
        .iter()
        .find(|t| {
            t.execute.as_ref().is_some_and(|a| {
                matches!(
                    a.effect.as_ref(),
                    Effect::ChooseObjectsIntoTrackedSet { .. }
                )
            })
        })
        .expect("an amplify ETB trigger must be synthesized")
}

/// CR 702.38a: `synthesize_amplify` builds an ETB trigger that reveals any
/// number of hand cards sharing a creature type, then puts N counters per card.
#[test]
fn synthesize_amplify_builds_etb_reveal_and_counter_trigger() {
    let mut face = face_with_amplify(1);
    synthesize_amplify(&mut face);

    let trigger = amplify_trigger(&face);
    assert!(matches!(trigger.mode, TriggerMode::ChangesZone));
    assert_eq!(trigger.destination, Some(Zone::Battlefield));
    assert_eq!(trigger.valid_card, Some(TargetFilter::SelfRef));

    let reveal = trigger.execute.as_ref().unwrap();
    // CR 702.38a: choose any number (min 0, max None) from the controller's hand.
    let Effect::ChooseObjectsIntoTrackedSet {
        chooser,
        filter,
        min,
        max,
    } = reveal.effect.as_ref()
    else {
        panic!("expected ChooseObjectsIntoTrackedSet");
    };
    assert_eq!(chooser, &TargetFilter::Controller);
    assert_eq!(*min, 0);
    assert_eq!(*max, None);
    // The reveal pool is the controller's hand, filtered by shared creature type.
    let props = match filter {
        TargetFilter::Typed(tf) => &tf.properties,
        other => panic!("expected typed hand filter, got {other:?}"),
    };
    assert!(props
        .iter()
        .any(|p| matches!(p, FilterProp::InZone { zone: Zone::Hand })));
    assert!(props
        .iter()
        .any(|p| matches!(p, FilterProp::SharesQuality { .. })));

    // CR 702.38a: enters with 1 +1/+1 counter per revealed card (TrackedSetSize).
    let put = reveal.sub_ability.as_ref().expect("counter sub-ability");
    let Effect::PutCounter {
        counter_type,
        count,
        target,
    } = put.effect.as_ref()
    else {
        panic!("expected PutCounter");
    };
    assert_eq!(*counter_type, CounterType::Plus1Plus1);
    assert_eq!(target, &TargetFilter::SelfRef);
    assert!(matches!(
        count,
        QuantityExpr::Ref {
            qty: QuantityRef::TrackedSetSize
        }
    ));
}

/// CR 702.38a: for N > 1 the per-card count is `N × revealed` (Multiply).
#[test]
fn synthesize_amplify_scales_counter_count_by_n() {
    let mut face = face_with_amplify(3);
    synthesize_amplify(&mut face);
    let put = amplify_trigger(&face)
        .execute
        .as_ref()
        .unwrap()
        .sub_ability
        .as_ref()
        .unwrap();
    let Effect::PutCounter { count, .. } = put.effect.as_ref() else {
        panic!("expected PutCounter");
    };
    assert!(
        matches!(count, QuantityExpr::Multiply { factor: 3, inner } if matches!(
            inner.as_ref(),
            QuantityExpr::Ref { qty: QuantityRef::TrackedSetSize }
        )),
        "Amplify 3 must put 3 counters per revealed card, got {count:?}"
    );
}

#[test]
fn synthesize_amplify_is_noop_without_keyword() {
    let mut face = CardFace::default();
    face.card_type.core_types.push(CoreType::Creature);
    synthesize_amplify(&mut face);
    assert!(face.triggers.is_empty());
}

#[test]
fn synthesize_amplify_is_idempotent() {
    let mut face = face_with_amplify(2);
    synthesize_amplify(&mut face);
    let after_first = face.triggers.len();
    synthesize_amplify(&mut face);
    assert_eq!(face.triggers.len(), after_first);
}

// ---------------------------------------------------------------------------
// End-to-end: ETB → reveal hand cards sharing a creature type → counters
// ---------------------------------------------------------------------------

fn main_phase_state() -> GameState {
    let mut state = GameState::new_two_player(42);
    state.active_player = PlayerId(0);
    state.phase = crate::types::phase::Phase::PreCombatMain;
    // CR 205.3m: `SharesQuality { CreatureType }` recognizes a subtype as a
    // creature type only if it's in the canonical list, which is empty in a bare
    // test state. Seed the types these tests use.
    state.all_creature_types = vec!["Goblin".to_string(), "Elf".to_string()];
    state
}

fn creature_card(
    state: &mut GameState,
    card: u64,
    owner: PlayerId,
    name: &str,
    subtype: &str,
    zone: Zone,
) -> ObjectId {
    let id = create_object(state, CardId(card), owner, name.to_string(), zone);
    let obj = state.objects.get_mut(&id).unwrap();
    obj.card_types.core_types.push(CoreType::Creature);
    obj.base_card_types.core_types.push(CoreType::Creature);
    obj.card_types.subtypes.push(subtype.to_string());
    obj.base_card_types.subtypes.push(subtype.to_string());
    id
}

fn attach_amplify(state: &mut GameState, id: ObjectId, n: u32, subtype: &str) {
    let mut face = face_with_amplify(n);
    face.card_type.subtypes.push(subtype.to_string());
    synthesize_amplify(&mut face);
    let obj = state.objects.get_mut(&id).unwrap();
    obj.trigger_definitions = face.triggers.clone().into();
    obj.base_trigger_definitions = Arc::new(face.triggers.clone());
}

/// CR 702.38a end-to-end (Amplify 2): the creature enters, the controller is
/// prompted to reveal hand cards that share its creature type (and only those),
/// and on selecting two it gains 2×2 = 4 +1/+1 counters.
#[test]
fn amplify_creature_enters_with_counters_per_revealed_sharing_card() {
    let mut state = main_phase_state();

    let amp = creature_card(
        &mut state,
        1,
        PlayerId(0),
        "Goblin Amplifier",
        "Goblin",
        Zone::Hand,
    );
    attach_amplify(&mut state, amp, 2, "Goblin");

    // Hand: two Goblins (share), one Elf (no share), plus an opponent Goblin.
    let g1 = creature_card(&mut state, 2, PlayerId(0), "Goblin A", "Goblin", Zone::Hand);
    let g2 = creature_card(&mut state, 3, PlayerId(0), "Goblin B", "Goblin", Zone::Hand);
    let _elf = creature_card(&mut state, 4, PlayerId(0), "Elf", "Elf", Zone::Hand);
    let _opp = creature_card(
        &mut state,
        5,
        PlayerId(1),
        "Foe Goblin",
        "Goblin",
        Zone::Hand,
    );

    let mut events = Vec::new();
    move_to_zone(&mut state, amp, Zone::Battlefield, &mut events);
    process_triggers(&mut state, &events);
    assert!(
        state
            .stack
            .iter()
            .any(|e| matches!(&e.kind, StackEntryKind::TriggeredAbility { .. })),
        "amplify ETB trigger must reach the stack"
    );

    resolve_top(&mut state, &mut Vec::new());
    let eligible = match &state.waiting_for {
        WaitingFor::ChooseObjectsSelection { eligible, .. } => eligible.clone(),
        other => panic!("expected ChooseObjectsSelection, got {other:?}"),
    };
    // CR 702.38a: only the controller's own hand cards sharing a creature type.
    assert!(eligible.contains(&TargetRef::Object(g1)));
    assert!(eligible.contains(&TargetRef::Object(g2)));
    assert_eq!(
        eligible.len(),
        2,
        "Elf (no share) and opponent's Goblin excluded"
    );

    apply_as_current(
        &mut state,
        GameAction::SelectTargets {
            targets: vec![TargetRef::Object(g1), TargetRef::Object(g2)],
        },
    )
    .expect("selection resolves");

    assert_eq!(
        state.objects[&amp].counters.get(&CounterType::Plus1Plus1),
        Some(&4),
        "Amplify 2 with two revealed cards = 4 +1/+1 counters"
    );
}

/// CR 702.38a: revealing nothing (the optional reveal) yields zero counters.
#[test]
fn amplify_revealing_nothing_adds_no_counters() {
    let mut state = main_phase_state();
    let amp = creature_card(
        &mut state,
        1,
        PlayerId(0),
        "Goblin Amplifier",
        "Goblin",
        Zone::Hand,
    );
    attach_amplify(&mut state, amp, 2, "Goblin");
    let _g1 = creature_card(&mut state, 2, PlayerId(0), "Goblin A", "Goblin", Zone::Hand);

    let mut events = Vec::new();
    move_to_zone(&mut state, amp, Zone::Battlefield, &mut events);
    process_triggers(&mut state, &events);
    resolve_top(&mut state, &mut Vec::new());
    assert!(matches!(
        state.waiting_for,
        WaitingFor::ChooseObjectsSelection { .. }
    ));

    apply_as_current(&mut state, GameAction::SelectTargets { targets: vec![] })
        .expect("declining the reveal resolves");
    assert_eq!(
        state.objects[&amp].counters.get(&CounterType::Plus1Plus1),
        None,
        "no reveal => no amplify counters"
    );
}

// ---------------------------------------------------------------------------
// Real-pipeline integration (MTGJSON -> parse -> synthesize)
// ---------------------------------------------------------------------------

/// Real Amplify card (Kilnmouth Dragon, Amplify 3) routed through
/// `build_oracle_face`.
#[test]
fn real_amplify_card_synthesizes_etb_trigger() {
    let atomic = AtomicCard {
        name: "Kilnmouth Dragon".to_string(),
        mana_cost: Some("{5}{R}{R}".to_string()),
        colors: vec!["R".to_string()],
        color_identity: vec!["R".to_string()],
        text: Some(
            "Amplify 3 (As this creature enters, put three +1/+1 counters on it for each Dragon \
             card you reveal in your hand.)\n\
             {T}: Kilnmouth Dragon deals damage equal to its power to any target."
                .to_string(),
        ),
        power: Some("2".to_string()),
        toughness: Some("2".to_string()),
        loyalty: None,
        defense: None,
        layout: "normal".to_string(),
        type_line: Some("Creature — Dragon".to_string()),
        types: vec!["Creature".to_string()],
        subtypes: vec!["Dragon".to_string()],
        supertypes: Vec::new(),
        keywords: Some(vec!["Amplify".to_string()]),
        side: None,
        face_name: None,
        mana_value: 7.0,
        legalities: Default::default(),
        leadership_skills: None,
        printings: Vec::new(),
        rulings: Vec::new(),
        is_game_changer: false,
        identifiers: AtomicIdentifiers {
            scryfall_oracle_id: Some("kilnmouth-dragon-oracle".to_string()),
            scryfall_id: Some("kilnmouth-dragon-face".to_string()),
        },
        foreign_data: Vec::new(),
    };

    let face = crate::database::synthesis::build_oracle_face(&atomic, None);
    assert!(
        face.keywords
            .iter()
            .any(|k| matches!(k, Keyword::Amplify(3))),
        "Amplify 3 must parse from MTGJSON"
    );
    let trigger = face
        .triggers
        .iter()
        .find(|t| {
            matches!(t.mode, TriggerMode::ChangesZone)
                && t.destination == Some(Zone::Battlefield)
                && t.execute.as_ref().is_some_and(|a| {
                    matches!(
                        a.effect.as_ref(),
                        Effect::ChooseObjectsIntoTrackedSet { .. }
                    )
                })
        })
        .expect("an Amplify ETB reveal/counter trigger must be synthesized");
    let _ = trigger;
    assert!(
        !crate::game::coverage::card_face_has_unimplemented_parts(&face),
        "face must have no Unimplemented parts after synthesis"
    );
}
