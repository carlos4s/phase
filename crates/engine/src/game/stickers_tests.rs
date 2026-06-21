use crate::game::effects;
use crate::game::layers::evaluate_layers;
use crate::game::scenario::{GameScenario, P0};
use crate::game::stickers::{apply_selected_sticker, available_sticker_candidates, set_player_sticker_sheets};
use crate::game::zones::move_to_zone;
use crate::types::ability::{Effect, QuantityExpr, ResolvedAbility, TargetFilter};
use crate::types::events::GameEvent;
use crate::types::keywords::Keyword;
use crate::types::player::PlayerCounterKind;
use crate::types::stickers::{AppliedSticker, StickerKind};
use crate::types::zones::Zone;

#[test]
fn stickers_modify_battlefield_object_and_public_zone_retention() {
    let mut scenario = GameScenario::new();
    let creature_id = scenario.add_creature(P0, "Bear", 2, 2).id();
    let mut game = scenario.build();
    let state = game.state_mut();

    set_player_sticker_sheets(
        state,
        P0,
        &[
            "Ancestral Hot Dog Minotaur".to_string(),
            "Playable Delusionary Hydra".to_string(),
        ],
    );
    state.players[0].add_player_counters(&PlayerCounterKind::Ticket, 20);

    let mut name = available_sticker_candidates(state, P0, Some(StickerKind::Name), None, false)
        .into_iter()
        .find(|candidate| {
            matches!(
                &candidate.sticker,
                AppliedSticker::Name { text, .. } if text == "Hot Dog"
            )
        })
        .expect("hot dog sticker available");
    if let AppliedSticker::Name { position, .. } = &mut name.sticker {
        *position = 1;
    }

    let flying = available_sticker_candidates(state, P0, Some(StickerKind::Ability), None, false)
        .into_iter()
        .find(|candidate| {
            matches!(
                &candidate.sticker,
                AppliedSticker::Ability { text, .. } if text == "Flying"
            )
        })
        .expect("flying sticker available");

    let pt = available_sticker_candidates(
        state,
        P0,
        Some(StickerKind::PowerToughness),
        Some(5),
        false,
    )
    .into_iter()
    .find(|candidate| {
        matches!(
            &candidate.sticker,
            AppliedSticker::PowerToughness {
                power: 8,
                toughness: 6,
                ..
            }
        )
    })
    .expect("8/6 sticker available");

    let mut events = Vec::new();
    apply_selected_sticker(state, creature_id, name.sticker, name.pay_ticket, &mut events);
    apply_selected_sticker(state, creature_id, flying.sticker, flying.pay_ticket, &mut events);
    apply_selected_sticker(state, creature_id, pt.sticker, pt.pay_ticket, &mut events);
    evaluate_layers(state);

    let creature = state.objects.get(&creature_id).unwrap();
    assert_eq!(creature.name, "Bear Hot Dog");
    assert_eq!(creature.power, Some(8));
    assert_eq!(creature.toughness, Some(6));
    assert!(creature.has_keyword(&Keyword::Flying));
    assert_eq!(creature.stickers.len(), 3);

    move_to_zone(state, creature_id, Zone::Graveyard, &mut events);
    let graveyard_creature = state.objects.get(&creature_id).unwrap();
    assert_eq!(graveyard_creature.zone, Zone::Graveyard);
    assert_eq!(graveyard_creature.name, "Bear Hot Dog");
    assert_eq!(graveyard_creature.power, Some(8));
    assert_eq!(graveyard_creature.toughness, Some(6));
    assert!(graveyard_creature.has_keyword(&Keyword::Flying));
    assert_eq!(graveyard_creature.stickers.len(), 3);

    move_to_zone(state, creature_id, Zone::Hand, &mut events);
    let hand_creature = state.objects.get(&creature_id).unwrap();
    assert_eq!(hand_creature.zone, Zone::Hand);
    assert_eq!(hand_creature.name, "Bear");
    assert_eq!(hand_creature.power, Some(2));
    assert_eq!(hand_creature.toughness, Some(2));
    assert!(!hand_creature.has_keyword(&Keyword::Flying));
    assert!(hand_creature.stickers.is_empty());
}

#[test]
fn put_sticker_effect_auto_applies_single_eligible_choice() {
    let mut scenario = GameScenario::new();
    let creature_id = scenario.add_creature(P0, "Turtle", 2, 2).id();
    let mut game = scenario.build();
    let state = game.state_mut();

    set_player_sticker_sheets(state, P0, &["Playable Delusionary Hydra".to_string()]);

    let ability = ResolvedAbility::new(
        Effect::PutSticker {
            target: TargetFilter::SpecificObject { id: creature_id },
            kind: Some(StickerKind::PowerToughness),
            count: 1,
            up_to: false,
            max_ticket_cost: Some(QuantityExpr::Fixed { value: 2 }),
            without_paying: true,
        },
        Vec::new(),
        creature_id,
        P0,
    );
    let mut events = Vec::<GameEvent>::new();
    effects::stickers::resolve(state, &ability, &mut events).unwrap();
    evaluate_layers(state);

    let creature = state.objects.get(&creature_id).unwrap();
    assert_eq!(creature.power, Some(1));
    assert_eq!(creature.toughness, Some(5));
    assert_eq!(creature.stickers.len(), 1);
}
