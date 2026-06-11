//! Runtime tests for Meld (CR 701.42 / CR 712.4). Declared from `game/mod.rs`
//! so the resolver (`game/meld.rs`) stays implementation-only.
//!
//! These drive the real resolve pipeline (`perform_meld` against a
//! `GameScenario`-built state) and would FAIL if the meld effect were reverted —
//! they are regression tests, not AST-shape tests. They exercise the building
//! block: exile-both → single melded permanent presenting the result face →
//! leave-split back to front faces → transform prohibition → ETB firing.

use std::sync::Arc;

use crate::game::meld::perform_meld;
use crate::game::scenario::{GameScenario, P0, P1};
use crate::types::ability::{Effect, PtValue, ResolvedAbility};
use crate::types::card::CardFace;
use crate::types::card_type::CoreType;
use crate::types::events::GameEvent;
use crate::types::identifiers::ObjectId;
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

const RESULT_NAME: &str = "Brisela, Voice of Nightmares";

/// Build a result `CardFace` (Brisela, 9/10 Legendary Angel Horror) and seed it
/// into the registry under its lowercase key (the path `walk_effect` →
/// `build_conjure_registry` populates in production).
fn seed_result_face(state: &mut crate::types::game_state::GameState) {
    let mut face = CardFace {
        name: RESULT_NAME.to_string(),
        power: Some(PtValue::Fixed(9)),
        toughness: Some(PtValue::Fixed(10)),
        ..CardFace::default()
    };
    face.card_type.core_types.push(CoreType::Creature);
    let registry = Arc::make_mut(&mut state.card_face_registry);
    registry.insert(RESULT_NAME.to_lowercase(), face);
}

/// A meld `ResolvedAbility` whose source is `source`, controlled by `controller`,
/// melding with `partner` into Brisela.
fn meld_ability(source: ObjectId, controller: PlayerId, partner: &str) -> ResolvedAbility {
    ResolvedAbility::new(
        Effect::Meld {
            partner: partner.to_string(),
            result: RESULT_NAME.to_string(),
        },
        Vec::new(),
        source,
        controller,
    )
}

/// Two co-owned/controlled meld halves on P0's battlefield, plus a seeded result
/// face. Returns `(state, source_id, partner_id)`.
fn both_halves() -> (crate::types::game_state::GameState, ObjectId, ObjectId) {
    let mut sc = GameScenario::new();
    let source = sc.add_creature(P0, "Gisela, the Broken Blade", 4, 3).id();
    let partner = sc.add_creature(P0, "Bruna, the Fading Light", 5, 4).id();
    seed_result_face(&mut sc.state);
    (sc.state, source, partner)
}

/// CR 701.42a / CR 712.4a: melding exiles both halves and puts a SINGLE melded
/// permanent onto the battlefield presenting the RESULT card's characteristics.
#[test]
fn meld_exiles_both_produces_single_permanent() {
    let (mut state, source, partner) = both_halves();
    let mut events = Vec::new();
    let ability = meld_ability(source, P0, "Bruna, the Fading Light");

    perform_meld(&mut state, &ability, &mut events).unwrap();

    // The survivor (source) is on the battlefield; the partner is no longer an
    // independent battlefield object.
    let survivor = state.objects.get(&source).expect("survivor exists");
    assert_eq!(survivor.zone, Zone::Battlefield);
    assert_eq!(
        survivor.merged_components,
        vec![source, partner],
        "the melded permanent records both halves"
    );
    assert!(
        !state.battlefield.iter().any(|&id| id == partner),
        "the partner half is absorbed into the melded permanent"
    );

    // CR 701.42a / CR 730.2: the partner is absorbed — it is NOT an independent
    // object in the exile list, yet its `zone` reads Battlefield (a component in
    // no zone list, mirroring merge_object_onto). On the pre-fix code the partner
    // was stranded in the exile list with zone == Exile, so all three of these
    // assertions fail without the absorption fix.
    let partner_obj = state.objects.get(&partner).expect("partner exists");
    assert_eq!(
        partner_obj.zone,
        Zone::Battlefield,
        "the absorbed partner's zone is Battlefield (component, not stranded in Exile)"
    );
    assert!(
        !state.exile.iter().any(|&id| id == partner),
        "the absorbed partner is NOT left in the exile zone list"
    );
    assert!(
        !state.battlefield.iter().any(|&id| id == partner),
        "the absorbed partner is a component, not an independent battlefield object"
    );

    // CR 712.4b: the melded permanent presents the RESULT card's characteristics
    // (Brisela 9/10) through the installed layer-1 copy effect.
    assert_eq!(survivor.name, RESULT_NAME);
    assert_eq!(survivor.power, Some(9));
    assert_eq!(survivor.toughness, Some(10));

    // CR 712.4b / CR 712.21: the survivor's BASE identity is NOT corrupted — it
    // is still its own front face (Gisela), so it returns correctly on leave.
    assert_eq!(survivor.base_name, "Gisela, the Broken Blade");
}

/// CR 712.21 / CR 712.4b: when the melded permanent leaves the battlefield, the
/// two cards return as their OWN FRONT FACES, each to its owner's graveyard.
#[test]
fn leave_split_returns_front_faces() {
    let (mut state, source, partner) = both_halves();
    let mut events = Vec::new();
    perform_meld(
        &mut state,
        &meld_ability(source, P0, "Bruna, the Fading Light"),
        &mut events,
    )
    .unwrap();

    // Destroy the melded permanent (battlefield → graveyard).
    let mut leave_events = Vec::new();
    crate::game::zones::move_to_zone(&mut state, source, Zone::Graveyard, &mut leave_events);

    let survivor = state
        .objects
        .get(&source)
        .expect("survivor object persists");
    assert_eq!(survivor.zone, Zone::Graveyard);
    // CR 712.4b: returns as its own front face, NOT as Brisela.
    assert_eq!(survivor.name, "Gisela, the Broken Blade");
    assert!(
        survivor.merged_components.is_empty(),
        "merge identity cleared on exit"
    );
    assert!(
        survivor.merge_kind.is_none(),
        "meld discriminator cleared on exit"
    );

    // CR 712.21: the partner card returns as its own front face, to its owner.
    let partner_obj = state.objects.get(&partner).expect("partner card returns");
    assert_eq!(partner_obj.zone, Zone::Graveyard);
    assert_eq!(partner_obj.name, "Bruna, the Fading Light");
    assert_eq!(partner_obj.owner, P0);

    // CR 701.42a / CR 730.2: the partner is single-listed in the graveyard and is
    // NOT double-listed in exile. On the pre-fix code the partner was stranded in
    // the exile list at meld time, so after the leave-split it remained in exile
    // AND was added to the graveyard — these two assertions catch that corruption.
    let p0_graveyard = &state
        .players
        .iter()
        .find(|p| p.id == P0)
        .expect("P0 exists")
        .graveyard;
    assert!(
        p0_graveyard.iter().any(|&id| id == partner),
        "the partner is listed in its owner's graveyard exactly once"
    );
    assert!(
        !state.exile.iter().any(|&id| id == partner),
        "the partner is NOT double-listed in exile after the leave-split"
    );
}

/// CR 701.42c: if the partner is absent (or not co-owned/controlled), the meld is
/// a no-op — the instigator stays on the battlefield, nothing is exiled.
#[test]
fn intervening_if_gates_both_ways() {
    // Partner ABSENT: only the source is on the battlefield.
    let mut sc = GameScenario::new();
    let source = sc.add_creature(P0, "Gisela, the Broken Blade", 4, 3).id();
    seed_result_face(&mut sc.state);
    let mut state = sc.state;
    let mut events = Vec::new();
    perform_meld(
        &mut state,
        &meld_ability(source, P0, "Bruna, the Fading Light"),
        &mut events,
    )
    .unwrap();

    let src = state.objects.get(&source).expect("source persists");
    assert_eq!(src.zone, Zone::Battlefield, "no-op: source stays put");
    assert!(src.merged_components.is_empty(), "no meld occurred");

    // Partner PRESENT but owned by a DIFFERENT player (controlled by P0 but not
    // owned) → still a no-op (CR 701.42b own AND control).
    let (mut state, source, _partner) = both_halves();
    // Re-own the partner to P1 while leaving control with P0.
    let partner2 = state
        .objects
        .iter()
        .find(|(_, o)| o.name == "Bruna, the Fading Light")
        .map(|(id, _)| *id)
        .unwrap();
    state.objects.get_mut(&partner2).unwrap().owner = P1;
    let mut events = Vec::new();
    perform_meld(
        &mut state,
        &meld_ability(source, P0, "Bruna, the Fading Light"),
        &mut events,
    )
    .unwrap();
    assert!(
        state
            .objects
            .get(&source)
            .unwrap()
            .merged_components
            .is_empty(),
        "CR 701.42b: a partner you control but don't own can't be melded"
    );
}

/// CR 712.4c: a melded permanent cannot be transformed — the instruction is a
/// silent no-op, and the permanent keeps presenting the result + its merge state.
#[test]
fn meld_permanent_cannot_transform() {
    let (mut state, source, _partner) = both_halves();
    let mut events = Vec::new();
    perform_meld(
        &mut state,
        &meld_ability(source, P0, "Bruna, the Fading Light"),
        &mut events,
    )
    .unwrap();

    // Attempt to transform the melded permanent — silent no-op (CR 712.4c).
    let mut t_events = Vec::new();
    crate::game::transform::transform_permanent(&mut state, source, &mut t_events).unwrap();

    let survivor = state.objects.get(&source).expect("survivor persists");
    assert_eq!(survivor.name, RESULT_NAME, "still presents the result");
    assert_eq!(
        survivor.merged_components,
        vec![source, _partner],
        "merge state intact after the ignored transform"
    );
}

/// CR 603.6a / CR 701.42a: melding emits a battlefield-entry `ZoneChanged` event
/// for the survivor (unlike Mutate, which suppresses ETB per CR 730.2b), so ETB
/// triggers can match the entering melded permanent.
#[test]
fn etb_fires_on_meld() {
    let (mut state, source, _partner) = both_halves();
    let mut events = Vec::new();
    perform_meld(
        &mut state,
        &meld_ability(source, P0, "Bruna, the Fading Light"),
        &mut events,
    )
    .unwrap();

    assert!(
        events.iter().any(|e| matches!(
            e,
            GameEvent::ZoneChanged { object_id, to: Zone::Battlefield, .. } if *object_id == source
        )),
        "the melded permanent's entry emits a battlefield ZoneChanged so ETB can fire"
    );
}
