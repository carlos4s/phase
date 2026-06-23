use crate::database::contraptions::synthesize_contraptions;
use crate::types::ability::{
    AbilityDefinition, AbilityKind, ControllerRef, Effect, TypeFilter, TypedFilter,
};
use crate::types::card::CardFace;
use crate::types::replacements::ReplacementEvent;

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
