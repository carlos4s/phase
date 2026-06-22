use crate::database::contraptions::synthesize_contraptions;
use crate::game::contraptions::{
    perform_contraption_upkeep_turn_based_action, resolve as resolve_contraptions,
};
use crate::game::deck_loading::create_contraption_deck_card;
use crate::game::printed_cards::apply_card_face_to_object;
use crate::game::zones::create_object;
use crate::types::ability::{
    AbilityDefinition, AbilityKind, Effect, QuantityExpr, ResolvedAbility,
};
use crate::types::card::CardFace;
use crate::types::card_type::CoreType;
use crate::types::events::GameEvent;
use crate::types::game_state::{GameState, WaitingFor};
use crate::types::identifiers::{CardId, ObjectId};
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

fn contraption_face(name: &str) -> CardFace {
    CardFace {
        name: name.to_string(),
        card_type: crate::types::card_type::CardType {
            supertypes: Vec::new(),
            core_types: vec![CoreType::Artifact],
            subtypes: vec!["Contraption".to_string()],
        },
        ..CardFace::default()
    }
}

fn rigger_face(name: &str) -> CardFace {
    CardFace {
        name: name.to_string(),
        card_type: crate::types::card_type::CardType {
            supertypes: Vec::new(),
            core_types: vec![CoreType::Creature],
            subtypes: vec!["Rigger".to_string()],
        },
        ..CardFace::default()
    }
}

fn steamflogger_boss_face() -> CardFace {
    let mut face = CardFace {
        name: "Steamflogger Boss".to_string(),
        oracle_text: Some(
            "If a Rigger you control would assemble a Contraption, it assembles two Contraptions instead."
                .to_string(),
        ),
        card_type: crate::types::card_type::CardType {
            supertypes: Vec::new(),
            core_types: vec![CoreType::Creature],
            subtypes: vec!["Goblin".to_string(), "Rigger".to_string()],
        },
        ..CardFace::default()
    };
    face.abilities.push(
        AbilityDefinition::new(
            AbilityKind::Spell,
            Effect::Unimplemented {
                name: "replacement_structure".to_string(),
                description: Some("replacement".to_string()),
            },
        )
        .description(
            "If a Rigger you control would assemble a Contraption, it assembles two Contraptions instead."
                .to_string(),
        ),
    );
    synthesize_contraptions(&mut face);
    face
}

#[test]
fn assemble_on_sprocket_moves_top_contraption_to_battlefield() {
    let mut state = GameState::new_two_player(1);
    let contraption_id = create_contraption_deck_card(
        &mut state,
        &contraption_face("Widget Contraption"),
        PlayerId(0),
    );

    let ability = ResolvedAbility::new(
        Effect::AssembleContraptionOnSprocket {
            sprocket: 2,
            remaining: 0,
        },
        Vec::new(),
        ObjectId(999),
        PlayerId(0),
    );
    let mut events = Vec::new();
    resolve_contraptions(&mut state, &ability, &mut events).unwrap();

    let object = state.objects.get(&contraption_id).unwrap();
    assert_eq!(object.zone, Zone::Battlefield);
    assert!(!object.in_contraption_deck);
    assert_eq!(object.contraption_sprocket, Some(2));
    assert!(events.iter().any(|event| {
        matches!(
            event,
            GameEvent::ContraptionAssembled {
                object_id,
                sprocket,
                ..
            } if *object_id == contraption_id && *sprocket == 2
        )
    }));
}

#[test]
fn upkeep_crank_prompts_for_current_sprocket_subset() {
    let mut state = GameState::new_two_player(1);
    let first = create_object(
        &mut state,
        CardId(1),
        PlayerId(0),
        "Hard Hat Area".to_string(),
        Zone::Battlefield,
    );
    let second = create_object(
        &mut state,
        CardId(2),
        PlayerId(0),
        "Widget Contraption".to_string(),
        Zone::Battlefield,
    );
    apply_card_face_to_object(
        state.objects.get_mut(&first).unwrap(),
        &contraption_face("Hard Hat Area"),
    );
    apply_card_face_to_object(
        state.objects.get_mut(&second).unwrap(),
        &contraption_face("Widget Contraption"),
    );
    state.objects.get_mut(&first).unwrap().contraption_sprocket = Some(1);
    state.objects.get_mut(&second).unwrap().contraption_sprocket = Some(2);

    let mut events = Vec::new();
    let prompt = perform_contraption_upkeep_turn_based_action(&mut state, &mut events);
    assert!(matches!(
        prompt,
        Some(WaitingFor::ChooseObjectsSelection { .. })
    ));
    match &state.waiting_for {
        WaitingFor::ChooseObjectsSelection { eligible, .. } => {
            assert_eq!(eligible.len(), 1);
            assert!(matches!(
                eligible[0],
                crate::types::ability::TargetRef::Object(id) if id == first
            ));
        }
        other => panic!("expected upkeep contraption selection, got {other:?}"),
    }
    assert_eq!(state.players[0].contraption_crank_sprocket, 1);
}

#[test]
fn steamflogger_boss_doubles_assemble_count() {
    let mut state = GameState::new_two_player(1);
    let boss = create_object(
        &mut state,
        CardId(1),
        PlayerId(0),
        "Steamflogger Boss".to_string(),
        Zone::Battlefield,
    );
    let rigger = create_object(
        &mut state,
        CardId(2),
        PlayerId(0),
        "Aerial Toastmaster".to_string(),
        Zone::Battlefield,
    );
    apply_card_face_to_object(
        state.objects.get_mut(&boss).unwrap(),
        &steamflogger_boss_face(),
    );
    apply_card_face_to_object(
        state.objects.get_mut(&rigger).unwrap(),
        &rigger_face("Aerial Toastmaster"),
    );
    create_contraption_deck_card(&mut state, &contraption_face("One"), PlayerId(0));
    create_contraption_deck_card(&mut state, &contraption_face("Two"), PlayerId(0));

    let ability = ResolvedAbility::new(
        Effect::AssembleContraptions {
            count: QuantityExpr::Fixed { value: 1 },
        },
        Vec::new(),
        rigger,
        PlayerId(0),
    );
    let mut events = Vec::new();
    resolve_contraptions(&mut state, &ability, &mut events).unwrap();

    match &state.waiting_for {
        WaitingFor::ChooseOneOfBranch { branches, .. } => match branches[0].effect.as_ref() {
            Effect::AssembleContraptionOnSprocket { remaining, .. } => assert_eq!(*remaining, 1),
            other => panic!("expected assemble branch, got {other:?}"),
        },
        other => panic!("expected sprocket choice, got {other:?}"),
    }
}
