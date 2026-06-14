//! Tests for the Archenemy runtime (CR 904 / CR 314). Declared from
//! `game/mod.rs` so `archenemy.rs` stays implementation-only (no inline tests).
//!
//! These drive the real pipeline: set-in-motion / abandon / SBA functions emit
//! events, the trigger machinery (`collect_triggers_into_deferred`) collects
//! them, and assertions check the resulting deferred-queue / command-zone /
//! event output. Several tests are deliberately discriminating: they fail if the
//! corresponding fix is reverted.

use super::archenemy::{
    abandon, active_schemes, check_scheme_abandon_sba, is_scheme_object, set_in_motion, top_scheme,
};
use crate::database::synthesis::synthesize_archenemy;
use crate::types::ability::{
    AbilityDefinition, AbilityKind, Effect, QuantityExpr, ResolvedAbility, StaticDefinition,
    TargetFilter, TriggerDefinition,
};
use crate::types::card::CardFace;
use crate::types::card_type::{CoreType, Supertype};
use crate::types::events::GameEvent;
use crate::types::game_state::{GameState, StackEntry, StackEntryKind};
use crate::types::identifiers::{CardId, ObjectId};
use crate::types::player::PlayerId;
use crate::types::statics::StaticMode;
use crate::types::triggers::TriggerMode;
use crate::types::zones::Zone;
use std::str::FromStr;

/// Build a `CardFace` for a scheme carrying the given triggers, statics, and
/// supertypes, then run `synthesize_archenemy` (the production stamping step) so
/// the trigger/static zones reflect the real card-build path.
fn synthesized_scheme_face(
    triggers: Vec<TriggerDefinition>,
    statics: Vec<StaticDefinition>,
    supertypes: Vec<Supertype>,
) -> CardFace {
    let mut face = CardFace::default();
    face.card_type.core_types.push(CoreType::Scheme);
    face.card_type.supertypes = supertypes;
    face.triggers = triggers;
    face.static_abilities = statics;
    synthesize_archenemy(&mut face);
    face
}

/// A `SetInMotion` trigger that draws a card.
fn set_in_motion_trigger() -> TriggerDefinition {
    TriggerDefinition::new(TriggerMode::SetInMotion)
        .valid_card(TargetFilter::SelfRef)
        .valid_target(TargetFilter::Controller)
        .execute(draw_ability())
}

/// An `Abandoned` trigger that draws a card.
fn abandoned_trigger() -> TriggerDefinition {
    TriggerDefinition::new(TriggerMode::Abandoned)
        .valid_card(TargetFilter::SelfRef)
        .valid_target(TargetFilter::Controller)
        .execute(draw_ability())
}

fn draw_ability() -> AbilityDefinition {
    AbilityDefinition::new(
        AbilityKind::Database,
        Effect::Draw {
            count: QuantityExpr::Fixed { value: 1 },
            target: TargetFilter::Controller,
        },
    )
}

/// Create a scheme object directly in `state.objects`, applying its synthesized
/// trigger/static definitions and setting its controller. Returns its id. The
/// object is NOT placed in any zone vector — the caller decides command zone vs
/// scheme deck.
fn create_scheme_object(
    state: &mut GameState,
    name: &str,
    face: &CardFace,
    controller: PlayerId,
) -> ObjectId {
    let id = ObjectId(state.next_object_id);
    state.next_object_id += 1;
    let mut obj = crate::game::game_object::GameObject::new(
        id,
        CardId(id.0),
        controller,
        name.to_string(),
        Zone::Command,
    );
    obj.controller = controller;
    obj.card_types = face.card_type.clone();
    for trig in &face.triggers {
        obj.trigger_definitions.push(trig.clone());
    }
    for st in &face.static_abilities {
        obj.static_definitions.push(st.clone());
    }
    state.objects.insert(id, obj);
    id
}

/// Place a face-down scheme deck (front = top), designate `archenemy`. Returns
/// the deck ids in order. Schemes are NOT placed in the command zone — the
/// scheme deck holds them face down until set in motion.
fn setup_scheme_deck(
    state: &mut GameState,
    archenemy: PlayerId,
    deck: &[(&str, &CardFace)],
) -> Vec<ObjectId> {
    let mut deck_ids = Vec::new();
    for (name, face) in deck {
        let id = create_scheme_object(state, name, face, archenemy);
        if let Some(obj) = state.objects.get_mut(&id) {
            obj.face_down = true;
        }
        state.scheme_deck.push_back(id);
        deck_ids.push(id);
    }
    state.archenemy = Some(archenemy);
    deck_ids
}

/// Place a single face-up scheme in the command zone (already set in motion),
/// designate `archenemy`. Returns its id.
fn setup_active_scheme(
    state: &mut GameState,
    archenemy: PlayerId,
    name: &str,
    face: &CardFace,
) -> ObjectId {
    let id = create_scheme_object(state, name, face, archenemy);
    if let Some(obj) = state.objects.get_mut(&id) {
        obj.face_down = false;
    }
    state.command_zone.push_back(id);
    state.archenemy = Some(archenemy);
    id
}

// ---------------------------------------------------------------------------
// 1. CoreType round-trip
// ---------------------------------------------------------------------------

#[test]
fn coretype_scheme_roundtrip() {
    // CR 314: Scheme is a nontraditional, non-permanent card type that offers no
    // protection quality.
    let s = CoreType::Scheme.to_string();
    assert_eq!(s, "Scheme");
    assert_eq!(CoreType::from_str(&s), Ok(CoreType::Scheme));
    // CR 314.2: not a permanent type.
    assert!(!CoreType::Scheme.is_permanent_type());
    assert_eq!(CoreType::Scheme.protection_quality_str(), None);
}

// ---------------------------------------------------------------------------
// 2. set_in_motion promotes the top scheme and fires its trigger (DISCRIMINATING)
// ---------------------------------------------------------------------------

#[test]
fn set_in_motion_promotes_top_and_fires_trigger() {
    // DISCRIMINATING: fails if `set_in_motion` stops turning the scheme face up /
    // stamping the controller, or if `match_set_in_motion` is reverted (the
    // SetInMotion trigger would never be collected).
    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    let non_arch = PlayerId(1);
    let scheme = synthesized_scheme_face(vec![set_in_motion_trigger()], vec![], vec![]);
    let deck_ids = setup_scheme_deck(&mut state, arch, &[("Scheme A", &scheme)]);
    let scheme_id = deck_ids[0];
    assert_eq!(top_scheme(&state), Some(scheme_id));
    // The scheme enters set_in_motion carrying a stale/foreign controller (the
    // non-archenemy player), so the CR 314.5 controller stamp must actively
    // correct it — this makes the controller assertion below discriminating.
    state.objects.get_mut(&scheme_id).unwrap().controller = non_arch;
    assert_ne!(
        state.objects.get(&scheme_id).unwrap().controller,
        arch,
        "scheme starts under a non-archenemy controller before set_in_motion"
    );

    let mut events = Vec::new();
    set_in_motion(&mut state, &mut events);

    // CR 904.9 / CR 701.32b: the scheme is now face up in the command zone.
    assert!(
        state.command_zone.contains(&scheme_id),
        "scheme moved into the command zone"
    );
    assert!(
        !state.objects.get(&scheme_id).unwrap().face_down,
        "scheme is face up"
    );
    assert!(state.scheme_deck.is_empty(), "scheme left the scheme deck");
    assert_eq!(active_schemes(&state), vec![scheme_id]);
    // CR 314.5: the archenemy is the controller of the face-up scheme.
    assert_eq!(
        state.objects.get(&scheme_id).unwrap().controller,
        arch,
        "archenemy stamped as the scheme's controller"
    );
    // CR 904.9: SchemeSetInMotion emitted, keyed to the scheme + archenemy.
    assert!(
        events.iter().any(|e| matches!(
            e,
            GameEvent::SchemeSetInMotion { scheme_id: s, player_id: p }
            if *s == scheme_id && *p == arch
        )),
        "SchemeSetInMotion event emitted, got {events:?}"
    );
    // CR 603.3: the SetInMotion trigger is collected into the deferred queue.
    assert!(
        state
            .deferred_triggers
            .iter()
            .any(|d| d.pending.source_id == scheme_id),
        "SetInMotion trigger from {scheme_id:?} must be collected, got {:?}",
        state.deferred_triggers
    );
}

// ---------------------------------------------------------------------------
// 3. set_in_motion is a no-op outside an Archenemy game
// ---------------------------------------------------------------------------

#[test]
fn set_in_motion_noop_outside_archenemy() {
    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    let scheme = synthesized_scheme_face(vec![], vec![], vec![]);
    let deck_ids = setup_scheme_deck(&mut state, arch, &[("Scheme A", &scheme)]);
    let scheme_id = deck_ids[0];
    // Not an Archenemy game.
    state.archenemy = None;

    let mut events = Vec::new();
    set_in_motion(&mut state, &mut events);

    assert_eq!(
        top_scheme(&state),
        Some(scheme_id),
        "scheme deck untouched when archenemy is None"
    );
    assert!(
        !state.command_zone.contains(&scheme_id),
        "scheme not promoted when archenemy is None"
    );
    assert!(events.is_empty(), "no events when archenemy is None");
}

// ---------------------------------------------------------------------------
// 4. precombat main sets the top scheme in motion (DISCRIMINATING)
// ---------------------------------------------------------------------------

#[test]
fn begin_precombat_main_sets_scheme_in_motion() {
    // DISCRIMINATING: fails if the `set_in_motion` hook is removed from
    // `finish_enter_phase`. Driving the phase machinery into PreCombatMain with
    // the active player = archenemy must set the top scheme in motion.
    use crate::types::phase::Phase;

    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    state.active_player = arch;
    let scheme = synthesized_scheme_face(vec![], vec![], vec![]);
    let deck_ids = setup_scheme_deck(&mut state, arch, &[("Scheme A", &scheme)]);
    let scheme_id = deck_ids[0];

    // Drive the real phase pipeline into PreCombatMain (Draw -> PreCombatMain).
    state.phase = Phase::Draw;
    let mut events = Vec::new();
    crate::game::turns::advance_phase(&mut state, &mut events);
    assert_eq!(state.phase, Phase::PreCombatMain);

    assert!(
        state.command_zone.contains(&scheme_id),
        "archenemy's precombat main set the top scheme in motion"
    );
    assert!(
        !state.objects.get(&scheme_id).unwrap().face_down,
        "scheme is face up after precombat main"
    );

    // A non-archenemy active player's precombat main does NOT set in motion.
    let mut state2 = GameState::new_two_player(7);
    let arch2 = PlayerId(0);
    let non_arch = PlayerId(1);
    state2.active_player = non_arch;
    let scheme2 = synthesized_scheme_face(vec![], vec![], vec![]);
    let deck_ids2 = setup_scheme_deck(&mut state2, arch2, &[("Scheme B", &scheme2)]);
    let scheme_id2 = deck_ids2[0];

    state2.phase = Phase::Draw;
    let mut events2 = Vec::new();
    crate::game::turns::advance_phase(&mut state2, &mut events2);
    assert_eq!(state2.phase, Phase::PreCombatMain);
    assert_eq!(
        top_scheme(&state2),
        Some(scheme_id2),
        "non-archenemy's precombat main must NOT set a scheme in motion"
    );
    assert!(
        !state2.command_zone.contains(&scheme_id2),
        "scheme stays in the deck on a non-archenemy turn"
    );
}

// ---------------------------------------------------------------------------
// 5. ongoing scheme static applies only while face up (DISCRIMINATING)
// ---------------------------------------------------------------------------

#[test]
fn ongoing_scheme_static_applies_only_while_face_up() {
    // DISCRIMINATING: fails if `synthesize_archenemy` stops stamping
    // `active_zones = [Command]` onto the scheme's static (the command-zone static
    // scan would never include it). After abandon the scheme leaves the active
    // command-zone view, so its static no longer applies.
    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    let mut scheme_static = StaticDefinition::new(StaticMode::Continuous);
    scheme_static.description = Some("scheme-static-marker".to_string());
    let scheme = synthesized_scheme_face(vec![], vec![scheme_static], vec![Supertype::Ongoing]);

    // Sanity: synthesis stamped the command zone onto the static.
    assert!(
        scheme.static_abilities[0]
            .active_zones
            .contains(&Zone::Command),
        "synthesize_archenemy must stamp Zone::Command on the scheme static"
    );

    let scheme_id = setup_active_scheme(&mut state, arch, "Ongoing Scheme", &scheme);

    // While face up in the command zone, the static is yielded by the real scan.
    let active_present = crate::game::functioning_abilities::game_active_statics(&state)
        .any(|(obj, _)| obj.id == scheme_id);
    assert!(
        active_present,
        "ongoing scheme static must apply while the scheme is face up"
    );

    // After abandon, the scheme is no longer in the command zone, so its static
    // no longer applies.
    let mut events = Vec::new();
    abandon(&mut state, scheme_id, &mut events);
    let still_present = crate::game::functioning_abilities::game_active_statics(&state)
        .any(|(obj, _)| obj.id == scheme_id);
    assert!(
        !still_present,
        "scheme static must NOT apply after the scheme is abandoned"
    );
}

// ---------------------------------------------------------------------------
// 6. non-ongoing scheme abandons on resolution (DISCRIMINATING)
// ---------------------------------------------------------------------------

#[test]
fn nonongoing_scheme_abandons_on_resolution() {
    // DISCRIMINATING: fails if `check_scheme_abandon_sba` no longer abandons a
    // face-up non-ongoing scheme, or if it stops respecting an on-stack scheme
    // trigger.
    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    state.active_player = arch;
    let scheme = synthesized_scheme_face(vec![], vec![], vec![]);
    let scheme_id = setup_active_scheme(&mut state, arch, "One-Shot Scheme", &scheme);

    // With a scheme trigger ON THE STACK, the SBA does nothing (CR 904.10).
    state.stack.push_back(StackEntry {
        id: ObjectId(99_999),
        source_id: scheme_id,
        controller: arch,
        kind: StackEntryKind::TriggeredAbility {
            source_id: scheme_id,
            ability: Box::new(ResolvedAbility::new(
                Effect::Draw {
                    count: QuantityExpr::Fixed { value: 1 },
                    target: TargetFilter::Controller,
                },
                vec![],
                scheme_id,
                arch,
            )),
            condition: None,
            trigger_event: None,
            description: None,
            source_name: String::new(),
            subject_match_count: None,
            die_result: None,
        },
    });
    let mut events = Vec::new();
    let mut any = false;
    check_scheme_abandon_sba(&mut state, &mut events, &mut any);
    assert!(
        !any,
        "no abandon while the scheme's ability is on the stack"
    );
    assert!(
        state.command_zone.contains(&scheme_id),
        "scheme still face up while its ability is on the stack"
    );

    // Clear the stack: now the SBA abandons the scheme.
    state.stack.clear();
    let mut events2 = Vec::new();
    let mut any2 = false;
    check_scheme_abandon_sba(&mut state, &mut events2, &mut any2);
    assert!(any2, "abandon once the ability leaves the stack");
    assert!(
        state.objects.get(&scheme_id).unwrap().face_down,
        "abandoned scheme is face down"
    );
    assert!(
        !state.command_zone.contains(&scheme_id),
        "abandoned scheme left the command zone"
    );
    assert_eq!(
        state.scheme_deck.back().copied(),
        Some(scheme_id),
        "abandoned scheme is on the bottom of the scheme deck"
    );
}

// ---------------------------------------------------------------------------
// 7. a deferred scheme trigger also blocks abandon (DISCRIMINATING)
// ---------------------------------------------------------------------------

#[test]
fn deferred_scheme_trigger_blocks_abandon() {
    // DISCRIMINATING (reviewer-requested negative): a scheme trigger sitting in
    // `deferred_triggers` (NOT yet on the stack) also blocks abandon — covers the
    // "waiting to be put on the stack" half of CR 904.10.
    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    state.active_player = arch;
    // A scheme whose SetInMotion trigger lands in the deferred queue.
    let scheme = synthesized_scheme_face(vec![set_in_motion_trigger()], vec![], vec![]);
    let deck_ids = setup_scheme_deck(&mut state, arch, &[("Trigger Scheme", &scheme)]);
    let scheme_id = deck_ids[0];

    // Set in motion: face up, and its SetInMotion trigger is now deferred.
    let mut events = Vec::new();
    set_in_motion(&mut state, &mut events);
    assert!(
        state
            .deferred_triggers
            .iter()
            .any(|d| d.pending.source_id == scheme_id),
        "scheme trigger must be deferred (waiting to be put on the stack)"
    );

    // The abandon SBA must do nothing while the trigger is waiting (CR 904.10).
    let mut events2 = Vec::new();
    let mut any = false;
    check_scheme_abandon_sba(&mut state, &mut events2, &mut any);
    assert!(
        !any,
        "no abandon while a scheme trigger is waiting to be put on the stack"
    );
    assert!(
        state.command_zone.contains(&scheme_id),
        "scheme stays face up while its trigger is deferred"
    );
}

// ---------------------------------------------------------------------------
// 8. ongoing scheme is not abandoned by the SBA (CR 904.11)
// ---------------------------------------------------------------------------

#[test]
fn ongoing_scheme_not_abandoned_by_sba() {
    // CR 904.11: an ongoing scheme is never abandoned by the SBA, even with the
    // stack clear.
    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    state.active_player = arch;
    let scheme = synthesized_scheme_face(vec![], vec![], vec![Supertype::Ongoing]);
    let scheme_id = setup_active_scheme(&mut state, arch, "Ongoing Scheme", &scheme);

    let mut events = Vec::new();
    let mut any = false;
    check_scheme_abandon_sba(&mut state, &mut events, &mut any);
    assert!(!any, "ongoing scheme must not be abandoned by the SBA");
    assert!(
        state.command_zone.contains(&scheme_id),
        "ongoing scheme stays face up in the command zone"
    );
    assert!(
        !state.objects.get(&scheme_id).unwrap().face_down,
        "ongoing scheme stays face up"
    );
}

// ---------------------------------------------------------------------------
// 9. abandon fires the Abandoned trigger
// ---------------------------------------------------------------------------

#[test]
fn abandon_fires_abandoned_trigger() {
    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    state.active_player = arch;
    let scheme = synthesized_scheme_face(vec![abandoned_trigger()], vec![], vec![]);
    let scheme_id = setup_active_scheme(&mut state, arch, "Abandon Scheme", &scheme);

    let mut events = Vec::new();
    abandon(&mut state, scheme_id, &mut events);

    // CR 701.33b: SchemeAbandoned emitted, keyed to the scheme.
    assert!(
        events.iter().any(|e| matches!(
            e,
            GameEvent::SchemeAbandoned { scheme_id: s, .. } if *s == scheme_id
        )),
        "SchemeAbandoned event emitted, got {events:?}"
    );
    // CR 603.3: the Abandoned trigger is collected into the deferred queue.
    assert!(
        state
            .deferred_triggers
            .iter()
            .any(|d| d.pending.source_id == scheme_id),
        "Abandoned trigger from {scheme_id:?} must be collected, got {:?}",
        state.deferred_triggers
    );
}

// ---------------------------------------------------------------------------
// 10. synthesize_archenemy appends Command, preserving pre-existing zones
// ---------------------------------------------------------------------------

#[test]
fn synthesize_archenemy_appends_command_zone() {
    // `synthesize_archenemy` must PUSH Zone::Command onto any pre-existing zone
    // list, not overwrite it, and be idempotent.
    let mut trigger = TriggerDefinition::new(TriggerMode::SetInMotion);
    trigger.trigger_zones = vec![Zone::Exile];
    let mut static_def = StaticDefinition::new(StaticMode::Continuous);
    static_def.active_zones = vec![Zone::Exile];

    let face = synthesized_scheme_face(vec![trigger], vec![static_def], vec![]);

    assert!(
        face.triggers[0].trigger_zones.contains(&Zone::Exile)
            && face.triggers[0].trigger_zones.contains(&Zone::Command),
        "pre-existing trigger zone preserved and Command appended, got {:?}",
        face.triggers[0].trigger_zones
    );
    assert!(
        face.static_abilities[0].active_zones.contains(&Zone::Exile)
            && face.static_abilities[0]
                .active_zones
                .contains(&Zone::Command),
        "pre-existing static zone preserved and Command appended, got {:?}",
        face.static_abilities[0].active_zones
    );

    // Idempotent: re-synthesis does not duplicate Command.
    let mut face2 = face;
    synthesize_archenemy(&mut face2);
    let command_count = face2.triggers[0]
        .trigger_zones
        .iter()
        .filter(|z| **z == Zone::Command)
        .count();
    assert_eq!(
        command_count, 1,
        "Command must not be duplicated on re-synthesis"
    );
}

// ---------------------------------------------------------------------------
// 11. archenemy = None skips the abandon SBA
// ---------------------------------------------------------------------------

#[test]
fn archenemy_none_skips_abandon_sba() {
    let mut state = GameState::new_two_player(7);
    let arch = PlayerId(0);
    state.active_player = arch;
    let scheme = synthesized_scheme_face(vec![], vec![], vec![]);
    let scheme_id = setup_active_scheme(&mut state, arch, "Scheme", &scheme);
    // Not an Archenemy game.
    state.archenemy = None;
    // Sanity: the object is recognized as a scheme regardless.
    assert!(is_scheme_object(&state, scheme_id));

    let mut events = Vec::new();
    let mut any = false;
    check_scheme_abandon_sba(&mut state, &mut events, &mut any);
    assert!(!any, "no abandon SBA work when archenemy is None");
    assert!(
        state.command_zone.contains(&scheme_id),
        "scheme untouched when there is no archenemy"
    );
    assert!(events.is_empty(), "no events when archenemy is None");
}
