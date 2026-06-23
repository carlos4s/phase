//! Unstable Contraption synthesis.

use crate::types::ability::{
    AbilityDefinition, AbilityKind, ControllerRef, Effect, QuantityExpr, ReplacementDefinition,
    ReplacementMode, TypeFilter, TypedFilter,
};
use crate::types::card::CardFace;
use crate::types::replacements::ReplacementEvent;

pub fn synthesize_contraptions(face: &mut CardFace) {
    if face.name == "Steamflogger Boss" {
        rewrite_steamflogger_boss(face);
    }
}

fn rewrite_steamflogger_boss(face: &mut CardFace) {
    face.abilities.retain(|ability| {
        !matches!(
            ability.effect.as_ref(),
            Effect::Unimplemented { name, .. } if name == "replacement_structure"
        )
    });

    if face
        .replacements
        .iter()
        .any(|replacement| replacement.event == ReplacementEvent::AssembleContraption)
    {
        return;
    }

    face.replacements.push(
        ReplacementDefinition::new(ReplacementEvent::AssembleContraption)
            .execute(AbilityDefinition::new(
                AbilityKind::Spell,
                Effect::AssembleContraptions {
                    count: QuantityExpr::Multiply {
                        factor: 2,
                        inner: Box::new(QuantityExpr::Ref {
                            qty: crate::types::ability::QuantityRef::EventContextAmount,
                        }),
                    },
                },
            ))
            .mode(ReplacementMode::Mandatory)
            .valid_card(
                TypedFilter::new(TypeFilter::Creature)
                    .subtype("Rigger".to_string())
                    .controller(ControllerRef::You)
                    .into(),
            )
            .description(
                "If a Rigger you control would assemble a Contraption, it assembles two Contraptions instead."
                    .to_string(),
            ),
    );
}
