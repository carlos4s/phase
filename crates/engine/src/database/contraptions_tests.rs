use crate::database::contraptions::synthesize_contraptions;
use crate::types::ability::{
    AbilityDefinition, AbilityKind, ControllerRef, Effect, QuantityExpr, TargetFilter, TypeFilter,
    TypedFilter,
};
use crate::types::card::CardFace;
use crate::types::card_type::{CardType, CoreType};
use crate::types::replacements::ReplacementEvent;

#[test]
fn synthesize_contraptions_rewrites_fixed_assemble_effect() {
    let mut face = CardFace {
        name: "Aerial Toastmaster".to_string(),
        oracle_text: Some("This creature assembles a Contraption.".to_string()),
        ..CardFace::default()
    };
    face.abilities.push(AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Unimplemented {
            name: "~".to_string(),
            description: Some("~ assembles a Contraption".to_string()),
        },
    ));

    synthesize_contraptions(&mut face);

    assert!(matches!(
        face.abilities[0].effect.as_ref(),
        Effect::AssembleContraptions {
            count: QuantityExpr::Fixed { value: 1 }
        }
    ));
}

#[test]
fn synthesize_contraptions_appends_then_assemble_follow_up() {
    let mut face = CardFace {
        name: "Spell Suck".to_string(),
        oracle_text: Some("Counter target spell, then assemble a Contraption.".to_string()),
        ..CardFace::default()
    };
    face.abilities.push(
        AbilityDefinition::new(
            AbilityKind::Spell,
            Effect::Counter {
                target: TargetFilter::StackSpell,
                source_rider: None,
                countered_spell_zone: None,
            },
        )
        .description("Counter target spell, then assemble a Contraption.".to_string()),
    );

    synthesize_contraptions(&mut face);

    let Some(sub_ability) = face.abilities[0].sub_ability.as_deref() else {
        panic!("expected assembled follow-up");
    };
    assert!(matches!(
        sub_ability.effect.as_ref(),
        Effect::AssembleContraptions {
            count: QuantityExpr::Fixed { value: 1 }
        }
    ));
}

#[test]
fn synthesize_contraptions_builds_steamflogger_boss_replacement() {
    let mut face = CardFace {
        name: "Steamflogger Boss".to_string(),
        oracle_text: Some(
            "If a Rigger you control would assemble a Contraption, it assembles two Contraptions instead."
                .to_string(),
        ),
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

    assert!(face.abilities.is_empty());
    assert!(face.replacements.iter().any(|replacement| {
        replacement.event == ReplacementEvent::AssembleContraption
            && replacement.valid_card
                == Some(
                    TypedFilter::new(TypeFilter::Creature)
                        .subtype("Rigger".to_string())
                        .controller(ControllerRef::You)
                        .into(),
                )
    }));
}

#[test]
fn synthesize_contraptions_rewrites_reassemble_target() {
    let mut face = CardFace {
        name: "Cogmentor".to_string(),
        oracle_text: Some("Reassemble target Contraption you control.".to_string()),
        card_type: CardType {
            supertypes: Vec::new(),
            core_types: vec![CoreType::Creature],
            subtypes: vec!["Rigger".to_string()],
        },
        ..CardFace::default()
    };
    face.abilities.push(AbilityDefinition::new(
        AbilityKind::Activated,
        Effect::Unimplemented {
            name: "reassemble".to_string(),
            description: Some("Reassemble target Contraption you control".to_string()),
        },
    ));

    synthesize_contraptions(&mut face);

    assert!(matches!(
        face.abilities[0].effect.as_ref(),
        Effect::ReassembleContraption {
            gain_control: false,
            ..
        }
    ));
}
