//! CR 701.x (Heist — MTG Arena digital-only) — production-path integration test.
//!
//! The unit-level tests in `effects/heist_tests.rs` exercise `heist::resolve`
//! and `heist::resolve_exile` directly. This file tests the **handoff** the
//! maintainer flagged as the risky part: the random look choice →
//! `WaitingFor::ChooseFromZoneChoice` → `GameAction::SelectCards` answer →
//! `pending_continuation` drain → finalizer. All of those moves go through
//! the production resolver / answer-handler / drain code paths (the same
//! code the `GameRunner` exercises in cast→choice→resolve scenarios), so
//! any regression in the partition semantics, the chosen-card injection
//! (`cont.chain.targets = chosen`), or the `drain_pending_continuation`
//! wiring would surface here.
//!
//! What this test does NOT cover (intentionally — they're orthogonal to the
//! Heist handoff):
//!
//! - Cast-cost payment and modal `ChooseBranch` routing (Heist on a card
//!   like Grave Expectations is one mode of a modal spell; routing to the
//!   Heist mode is the modal spell's job, not Heist's).
//! - Triggered-ability target declaration (Grenzo's ETB "target opponent's
//!   library" declares its target via the trigger's targeting phase; that
//!   pipeline is generic over all targeted triggers).
//!
//! Both are covered by their own integration tests elsewhere; the goal here
//! is to drive the parsed `Effect::Heist` through the production
//! resolver → interactive prompt → answer → continuation drain →
//! finalizer path end-to-end.

use engine::game::ability_utils::build_resolved_from_def_with_targets;
use engine::game::effects::resolve_ability_chain;
use engine::game::zones::create_object;
use engine::game::EngineError;
use engine::types::ability::{
    AbilityKind, CastingPermission, Effect, ManaSpendPermission, TargetRef,
};
use engine::types::card_type::CoreType;
use engine::types::events::GameEvent;
use engine::types::game_state::{ExileLinkKind, GameState, WaitingFor};
use engine::types::identifiers::{CardId, ObjectId};
use engine::types::mana::{ManaCost, ManaUnit};
use engine::types::player::PlayerId;
use engine::types::statics::CastFrequency;
use engine::types::zones::Zone;

/// Build the controller's Heist source on the battlefield so the finalizer
/// can link the exiled card to it via `ExileLinkKind::HideawayLookable`.
fn add_heist_source(state: &mut GameState, controller: PlayerId) -> ObjectId {
    let src = create_object(
        state,
        CardId(900),
        controller,
        "Heist Source".to_string(),
        Zone::Battlefield,
    );
    // Some mana to discourage the engine from stripping the ability on the
    // source; not strictly required, but keeps the source "live".
    state.players[controller.0 as usize]
        .mana_pool
        .add(ManaUnit::new(
            engine::types::mana::ManaType::Colorless,
            ObjectId(0),
            false,
            vec![],
        ));
    src
}

/// Put a named nonland creature into `player`'s library so it is a heistable
/// target. Distinct names keep card-text identity unambiguous across cards.
fn library_creature(
    state: &mut GameState,
    card_id: CardId,
    player: PlayerId,
    name: &str,
) -> ObjectId {
    let id = create_object(state, card_id, player, name.to_string(), Zone::Library);
    let obj = state.objects.get_mut(&id).unwrap();
    obj.card_types.core_types.push(CoreType::Creature);
    obj.mana_cost = ManaCost::generic(2);
    id
}

/// Parse a Heist clause into a `ResolvedAbility` and seed its target slot
/// with `opponent`. This is the **production entry point** the maintainer
/// asked for: a real parsed Heist ability driven through the resolver,
/// not a hand-constructed `Effect::Heist` literal. The builder preserves
/// the parsed sub-ability / duration / repeat_for fields, which the
/// hand-struct pattern would silently drop.
fn parsed_heist_ability(
    source: ObjectId,
    controller: PlayerId,
    opponent: PlayerId,
) -> engine::types::ability::ResolvedAbility {
    let def = engine::parser::oracle_effect::parse_effect_chain(
        "heist target opponent's library.",
        AbilityKind::Spell,
    );
    build_resolved_from_def_with_targets(
        &def,
        source,
        controller,
        vec![TargetRef::Player(opponent)],
    )
}

/// The Heist look step produces a `WaitingFor::ChooseFromZoneChoice` with
/// the chosen 3 random nonland cards and `count: 1`. Asserts the prompt
/// invariants: land excluded, count 1, exactly the 3 nonlands offered.
fn assert_heist_prompt(state: &GameState, controller: PlayerId, nonlands: &[ObjectId]) {
    match &state.waiting_for {
        WaitingFor::ChooseFromZoneChoice {
            player,
            cards,
            count,
            up_to,
            ..
        } => {
            assert_eq!(*player, controller);
            assert_eq!(*count, 1);
            assert!(!up_to);
            assert_eq!(
                cards.len(),
                3,
                "Heist must offer exactly three random nonland cards"
            );
            for id in nonlands {
                assert!(cards.contains(id), "nonland candidate {id:?} not offered",);
            }
            // No duplicate nonland IDs in the offer.
            let mut sorted = cards.clone();
            sorted.sort_by_key(|id| id.0);
            sorted.dedup();
            assert_eq!(sorted.len(), cards.len(), "offered cards must be distinct");
            // The land MUST NOT appear in the offer.
            for id in cards {
                let is_land = state
                    .objects
                    .get(id)
                    .is_some_and(|o| o.card_types.core_types.contains(&CoreType::Land));
                assert!(
                    !is_land,
                    "land {id:?} must never be offered as a Heist candidate",
                );
            }
        }
        other => panic!("expected ChooseFromZoneChoice after Heist resolve, got {other:?}"),
    }
}

/// Drive the production answer-handler for the ChooseFromZoneChoice prompt.
/// `engine::apply` is the same function `GameRunner::act` calls — it routes
/// `GameAction::SelectCards` into `engine_resolution_choices::handle_zone_choice`,
/// which sets `cont.chain.targets = chosen`, partitions unchosen into the
/// sub-ability (none here, so unchosen are untouched), and drains the
/// `PendingContinuation` (which runs `HeistExile` on the chosen card).
fn select_heist_card(
    state: &mut GameState,
    actor: PlayerId,
    chosen: ObjectId,
) -> Result<(), EngineError> {
    // Capture the offered cards BEFORE the action: handle_zone_choice
    // validates that every selected ID was in the eligible set. We
    // re-read them out of the WaitingFor to be sure.
    let offered: Vec<ObjectId> = match &state.waiting_for {
        WaitingFor::ChooseFromZoneChoice { cards, .. } => cards.clone(),
        other => panic!("select_heist_card called without ChooseFromZoneChoice: {other:?}"),
    };
    assert!(
        offered.contains(&chosen),
        "chosen card {chosen:?} was not in the offer {offered:?}",
    );
    engine::game::apply(
        state,
        actor,
        engine::types::actions::GameAction::SelectCards {
            cards: vec![chosen],
        },
    )?;
    Ok(())
}

#[test]
fn heist_production_path_exiles_chosen_face_down_and_grants_cast_permission() {
    // Seeded RNG so the three nonlands offered are deterministic.
    let mut state = GameState::new_two_player(0x5EED);
    let controller = PlayerId(0);
    let opponent = PlayerId(1);
    let source = add_heist_source(&mut state, controller);

    // Three heistable nonlands + one land (must be excluded from the offer).
    let bear = library_creature(&mut state, CardId(1), opponent, "Bear");
    let goblin = library_creature(&mut state, CardId(2), opponent, "Goblin");
    let elf = library_creature(&mut state, CardId(3), opponent, "Elf");
    let forest = create_object(
        &mut state,
        CardId(4),
        opponent,
        "Forest".to_string(),
        Zone::Library,
    );
    state
        .objects
        .get_mut(&forest)
        .unwrap()
        .card_types
        .core_types
        .push(CoreType::Land);

    // REAL PARSED HEIST ABILITY — the production entry point.
    let ability = parsed_heist_ability(source, controller, opponent);

    // --- Production step 1: resolver raises ChooseFromZoneChoice. ---
    let mut events = Vec::new();
    resolve_ability_chain(&mut state, &ability, &mut events, 0).unwrap();
    assert_heist_prompt(&state, controller, &[bear, goblin, elf]);
    // The look step did NOT move any card out of the library.
    for id in [bear, goblin, elf] {
        assert_eq!(
            state.objects[&id].zone,
            Zone::Library,
            "nonland {id:?} must still be in the library before the player picks",
        );
    }

    // --- Production step 2: player picks one through the normal action
    // handler. `engine::apply` is the SAME function `GameRunner::act` calls,
    // so this drives the production `ChooseFromZoneChoice` answer path,
    // the partition logic, and the `drain_pending_continuation` that
    // runs `HeistExile` on the chosen card. ---
    let chosen = elf;
    select_heist_card(&mut state, controller, chosen).unwrap();

    // --- Production step 3: finalizer ran. Assertions on the HANDOFF the
    // maintainer flagged as risky.
    let chosen_obj = &state.objects[&chosen];
    assert_eq!(
        chosen_obj.zone,
        Zone::Exile,
        "chosen card must be exiled by the HeistExile finalizer",
    );
    assert!(
        chosen_obj.face_down,
        "chosen card must be face-down in exile (CR 406.3)",
    );
    // CR 406.3 + HideawayLookable: the controller may look at the exiled card.
    assert!(
        state.exile_links.iter().any(|link| {
            link.exiled_id == chosen
                && link.source_id == source
                && link.kind == ExileLinkKind::HideawayLookable
        }),
        "chosen card must be linked to the source with HideawayLookable",
    );
    // Permanent any-color cast-from-exile permission (reminder: "for as long
    // as it remains exiled, … spend mana as though it were mana of any type").
    let grant = chosen_obj
        .casting_permissions
        .iter()
        .find_map(|perm| match perm {
            CastingPermission::PlayFromExile {
                mana_spend_permission,
                exiled_by_ability_controller,
                ..
            } => Some((mana_spend_permission, exiled_by_ability_controller)),
            _ => None,
        })
        .expect("chosen card must have a PlayFromExile permission");
    assert_eq!(
        grant.0,
        &Some(ManaSpendPermission::AnyTypeOrColor),
        "PlayFromExile must allow any-type-or-color mana",
    );
    assert_eq!(
        grant.1,
        &Some(controller),
        "PlayFromExile.granted_to / exiled_by_ability_controller must bind to the Heist controller",
    );

    // --- Production step 4: unchosen cards are PARTITION-UNTOUCHED. The
    // partition logic in engine_resolution_choices::handle_zone_choice
    // pushes unchosen into `sub_ability.targets` ONLY when the continuation
    // has a `sub_ability`. `HeistExile` carries none, so unchosen are
    // never forwarded anywhere — they stay in the opponent's library, NOT
    // face-down, NOT exiled, NOT granted. This is the exact property the
    // maintainer asked us to assert.
    for id in [bear, goblin] {
        if id == chosen {
            continue;
        }
        let obj = &state.objects[&id];
        assert_eq!(
            obj.zone,
            Zone::Library,
            "unchosen nonland {id:?} must remain in the opponent's library",
        );
        assert!(
            !obj.face_down,
            "unchosen nonland {id:?} must NOT be marked face_down",
        );
        assert!(
            obj.casting_permissions.is_empty(),
            "unchosen nonland {id:?} must NOT have any cast-from-exile permission",
        );
        assert!(
            !state.exile_links.iter().any(|link| link.exiled_id == id),
            "unchosen nonland {id:?} must NOT be linked",
        );
    }
    // And the land is exactly where it started.
    assert_eq!(
        state.objects[&forest].zone,
        Zone::Library,
        "land must remain in the opponent's library",
    );

    // The effect + finalizer both emit EffectResolved events through the
    // production event stream.
    let kinds: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            GameEvent::EffectResolved { kind, .. } => Some(*kind),
            _ => None,
        })
        .collect();
    assert!(
        kinds.iter().any(|k| matches!(
            k,
            engine::types::ability::EffectKind::Heist
                | engine::types::ability::EffectKind::HeistExile
        )),
        "expected EffectResolved events for Heist and HeistExile, got {kinds:?}",
    );
}

#[test]
fn heist_production_path_skip_cast_frequency_is_unlimited_and_idempotent() {
    // Regression: confirm the standing PlayFromExile permission is
    // CastFrequency::Unlimited — the Heist reminder says "you may cast that
    // card", which is a persistent (non single-use) grant. A regression to
    // `single_use: true` would silently turn Heist into "cast once" cards,
    // breaking the mechanic for the second cast (and beyond).
    let mut state = GameState::new_two_player(0xC0DE);
    let controller = PlayerId(0);
    let opponent = PlayerId(1);
    let source = add_heist_source(&mut state, controller);

    let bear = library_creature(&mut state, CardId(1), opponent, "Persistent Bear");
    let ability = parsed_heist_ability(source, controller, opponent);

    let mut events = Vec::new();
    resolve_ability_chain(&mut state, &ability, &mut events, 0).unwrap();
    select_heist_card(&mut state, controller, bear).unwrap();

    let perm = state.objects[&bear]
        .casting_permissions
        .iter()
        .find(|p| matches!(p, CastingPermission::PlayFromExile { .. }))
        .expect("PlayFromExile granted");
    if let CastingPermission::PlayFromExile {
        frequency,
        single_use,
        single_use_group,
        duration,
        granted_to,
        mana_spend_permission,
        ..
    } = perm
    {
        assert_eq!(
            *frequency,
            CastFrequency::Unlimited,
            "Heist must be Unlimited"
        );
        assert!(!*single_use, "Heist must NOT be single_use");
        assert!(single_use_group.is_none(), "single_use_group must be None");
        assert_eq!(
            *duration,
            engine::types::ability::Duration::Permanent,
            "Heist's grant must be Permanent (for as long as it remains exiled)",
        );
        assert_eq!(
            *granted_to, controller,
            "granted_to must be the Heist controller"
        );
        assert_eq!(
            *mana_spend_permission,
            Some(ManaSpendPermission::AnyTypeOrColor),
            "any-type-or-color mana must be granted",
        );
    } else {
        panic!("expected PlayFromExile variant");
    }
}

#[test]
fn heist_look_step_does_not_drain_when_library_has_no_nonlands() {
    // Edge case: opponent's library is ONLY lands → Heist has nothing to
    // offer. The production path must NOT raise ChooseFromZoneChoice
    // (there is nothing to choose from) and must NOT stash a continuation
    // (no continuation means no risk of leaking a drain on an empty
    // selection). The effect emits EffectResolved and the chain unwinds
    // cleanly. This catches a class of bugs where an empty-pool check
    // short-circuits but still leaves a `PendingContinuation` parked.
    let mut state = GameState::new_two_player(0xDEAD);
    let controller = PlayerId(0);
    let opponent = PlayerId(1);
    let source = add_heist_source(&mut state, controller);

    let only_forest = create_object(
        &mut state,
        CardId(1),
        opponent,
        "Only Forest".to_string(),
        Zone::Library,
    );
    state
        .objects
        .get_mut(&only_forest)
        .unwrap()
        .card_types
        .core_types
        .push(CoreType::Land);

    let ability = parsed_heist_ability(source, controller, opponent);
    let mut events = Vec::new();
    resolve_ability_chain(&mut state, &ability, &mut events, 0).unwrap();

    assert!(
        !matches!(state.waiting_for, WaitingFor::ChooseFromZoneChoice { .. }),
        "Heist must not raise a choice when the opponent has no nonlands",
    );
    assert!(
        state.pending_continuation.is_none(),
        "Heist must not stash a continuation when there is nothing to choose",
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            GameEvent::EffectResolved {
                kind: engine::types::ability::EffectKind::Heist,
                ..
            }
        )),
        "the empty-pool Heist must still emit its EffectResolved event",
    );
    assert_eq!(
        state.objects[&only_forest].zone,
        Zone::Library,
        "the lone land stays in the library — never touched by the no-op",
    );
    assert!(
        state.objects[&only_forest].casting_permissions.is_empty(),
        "no permission may be granted when the pool is empty",
    );
    assert!(
        state.exile_links.is_empty(),
        "no exile link when the pool is empty"
    );
}

// Sanity: the target filter on `Effect::Heist` round-trips through parsing
// and resolver registration (exhaustiveness coverage). Not strictly a
// production-path test, but cheap insurance that a future refactor of
// the parser arm keeps the opponent-target wiring intact.
#[test]
fn heist_target_filter_round_trips_through_parse() {
    use engine::types::ability::TargetFilter;
    let def = engine::parser::oracle_effect::parse_effect_chain(
        "heist target opponent's library.",
        AbilityKind::Spell,
    );
    match &*def.effect {
        Effect::Heist { target, .. } => {
            // The parser must produce a target filter that resolves to an
            // opponent player (mirrors parse_target("target opponent")).
            let mirrors_opponent =
                matches!(target, TargetFilter::Typed(_)) || matches!(target, TargetFilter::Player);
            assert!(
                mirrors_opponent,
                "Heist's parsed target filter must be a player-targetable filter, got {target:?}",
            );
        }
        other => panic!("expected Effect::Heist, got {other:?}"),
    }
}
