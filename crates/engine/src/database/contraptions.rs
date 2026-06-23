//! Unstable Contraption / Assemble synthesis.

use crate::types::ability::{
    AbilityDefinition, AbilityKind, ControllerRef, Effect, FilterProp, QuantityExpr, QuantityRef,
    ReassembleControlMode, ReplacementDefinition, ReplacementMode, TargetFilter, TypeFilter,
    TypedFilter,
};
use crate::types::card::CardFace;
use crate::types::replacements::ReplacementEvent;
use crate::types::zones::Zone;
use crate::types::TriggerDefinition;

pub fn synthesize_contraptions(face: &mut CardFace) {
    let oracle = face.oracle_text.clone().unwrap_or_default();
    let mentions_contraptions = oracle.contains("Contraption")
        || face
            .card_type
            .subtypes
            .iter()
            .any(|subtype| subtype.eq_ignore_ascii_case("Contraption"))
        || face.triggers.iter().any(|trigger| {
            matches!(
                trigger.mode,
                crate::types::triggers::TriggerMode::CrankContraption
            )
        });
    if !mentions_contraptions {
        return;
    }

    for ability in &mut face.abilities {
        rewrite_ability(ability);
    }
    for TriggerDefinition { execute, .. } in &mut face.triggers {
        if let Some(ability) = execute.as_deref_mut() {
            rewrite_ability(ability);
        }
    }

    if face.name == "Steamflogger Boss" {
        rewrite_steamflogger_boss(face);
    }
}

fn rewrite_ability(ability: &mut AbilityDefinition) {
    if let Some(effect) = synthesized_effect_for_unimplemented(ability.effect.as_ref()) {
        *ability.effect = effect;
    }

    if ability.sub_ability.is_none() {
        if let Some(sub_ability) = assemble_follow_up_from_description(ability) {
            ability.sub_ability = Some(Box::new(sub_ability));
        }
    }

    if let Some(sub_ability) = ability.sub_ability.as_deref_mut() {
        rewrite_ability(sub_ability);
    }
    if let Some(else_ability) = ability.else_ability.as_deref_mut() {
        rewrite_ability(else_ability);
    }
    for branch in &mut ability.mode_abilities {
        rewrite_ability(branch);
    }
}

fn synthesized_effect_for_unimplemented(effect: &Effect) -> Option<Effect> {
    let Effect::Unimplemented {
        description: Some(description),
        ..
    } = effect
    else {
        return None;
    };

    let description = description.trim();
    parse_assemble_effect(description).or_else(|| parse_reassemble_effect(description))
}

fn assemble_follow_up_from_description(ability: &AbilityDefinition) -> Option<AbilityDefinition> {
    let description = ability.description.as_deref()?.trim();
    let assemble_tail = if description.ends_with("then assemble a Contraption.")
        || description.ends_with("Assemble a Contraption.")
    {
        Some(fixed(1))
    } else {
        None
    }?;

    if matches!(
        ability.effect.as_ref(),
        Effect::AssembleContraptions { .. }
            | Effect::AssembleContraptionsFromRollDifference
            | Effect::AssembleContraptionOnSprocket { .. }
    ) {
        return None;
    }

    let mut sub_ability = AbilityDefinition::new(
        AbilityKind::Spell,
        Effect::AssembleContraptions {
            count: assemble_tail,
        },
    );
    sub_ability.sub_link = crate::types::ability::SubAbilityLink::SequentialSibling;
    Some(sub_ability)
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
                        inner: Box::new(event_amount()),
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

fn controlled_battlefield_contraption_filter() -> TargetFilter {
    contraption_filter()
        .controller(ControllerRef::You)
        .properties(vec![FilterProp::InZone {
            zone: Zone::Battlefield,
        }])
        .into()
}

fn contraption_filter() -> TypedFilter {
    TypedFilter::new(TypeFilter::Subtype("Contraption".to_string()))
}

fn fixed(value: i32) -> QuantityExpr {
    QuantityExpr::Fixed { value }
}

fn x_quantity() -> QuantityExpr {
    QuantityExpr::Ref {
        qty: QuantityRef::Variable {
            name: "X".to_string(),
        },
    }
}

fn event_amount() -> QuantityExpr {
    QuantityExpr::Ref {
        qty: QuantityRef::EventContextAmount,
    }
}

fn parse_assemble_effect(description: &str) -> Option<Effect> {
    let body = strip_assemble_subject(description)?;
    let count = match body {
        "a Contraption" => Some(fixed(1)),
        "two Contraptions" => Some(fixed(2)),
        "X Contraptions" => Some(x_quantity()),
        "X plus one Contraptions" => Some(QuantityExpr::Offset {
            inner: Box::new(x_quantity()),
            offset: 1,
        }),
        "a number of Contraptions equal to the result" => Some(event_amount()),
        "a Contraption for each Contraption you control" => Some(QuantityExpr::Ref {
            qty: QuantityRef::ObjectCount {
                filter: controlled_battlefield_contraption_filter(),
            },
        }),
        "a number of Contraptions equal to the difference between those results" => None,
        _ => return None,
    };

    if body == "a number of Contraptions equal to the difference between those results" {
        return Some(Effect::AssembleContraptionsFromRollDifference);
    }

    Some(Effect::AssembleContraptions { count: count? })
}

fn strip_assemble_subject(description: &str) -> Option<&str> {
    [
        "~ assembles ",
        "it assembles ",
        "This Contraption assembles ",
        "Assemble ",
    ]
    .into_iter()
    .find_map(|prefix| description.strip_prefix(prefix))
}

fn parse_reassemble_effect(description: &str) -> Option<Effect> {
    if let Some(body) = description.strip_prefix("Reassemble ") {
        return Some(Effect::ReassembleContraption {
            target: parse_reassemble_target(body)?,
            control_mode: ReassembleControlMode::KeepController,
        });
    }
    if let Some(body) = description.strip_prefix("it reassembles ") {
        return Some(Effect::ReassembleContraption {
            target: parse_reassemble_target(body)?,
            control_mode: ReassembleControlMode::GainControl,
        });
    }
    None
}

fn parse_reassemble_target(description: &str) -> Option<TargetFilter> {
    match description {
        "target Contraption you control" => {
            Some(contraption_filter().controller(ControllerRef::You).into())
        }
        "target Contraption that player controls" => Some(
            contraption_filter()
                .controller(ControllerRef::TriggeringPlayer)
                .into(),
        ),
        _ => None,
    }
}
