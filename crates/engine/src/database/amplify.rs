//! Amplify (CR 702.38a) — enters-with-counters keyed off a hand reveal.
//!
//! CR 702.38a: "Amplify N" is a static ability that means "As this object
//! enters, reveal any number of cards from your hand that share a creature type
//! with it. This permanent enters with N +1/+1 counters on it for each card
//! revealed this way. You can't reveal this card or any other cards that are
//! entering the battlefield at the same time as this card."
//!
//! Composed entirely from existing building blocks — this module adds **no new
//! effect**:
//!
//! 1. An enters-the-battlefield trigger (`TriggerMode::ChangesZone`,
//!    `destination = Battlefield`, `valid_card = SelfRef`).
//! 2. Whose effect is `Effect::ChooseObjectsIntoTrackedSet` over the controller's
//!    **hand**, filtered to cards that share a creature type with the entering
//!    permanent — the "reveal any number" optional multi-select (`min = 0`,
//!    `max = None`). The chosen cards become the chain's tracked set.
//! 3. Chained to a `sub_ability` `Effect::PutCounter` that places
//!    `N × tracked-set-size` +1/+1 counters on the permanent
//!    (`QuantityRef::TrackedSetSize`, scaled by `QuantityExpr::Multiply` for
//!    `N > 1`).
//!
//! "You can't reveal this card or other simultaneous entrants" is satisfied for
//! free: by the time the trigger resolves the source has left the hand (it is on
//! the battlefield), and other entrants are likewise no longer in hand, so the
//! hand-scoped filter never offers them.
//!
//! Modeling note: CR 702.38a is an as-enters static; the engine has no
//! interactive-replacement path (a `ReplacementDefinition` cannot pause for a
//! player choice mid-zone-change), so the reveal-and-count is modeled as an ETB
//! trigger. For every Amplify card the base power/toughness is positive, so the
//! brief window before the counters land carries no state-based-action
//! consequence — the observable result is identical to the as-enters wording.

use crate::types::ability::{
    AbilityDefinition, AbilityKind, ControllerRef, Effect, FilterProp, QuantityExpr, QuantityRef,
    SharedQuality, SharedQualityRelation, TargetFilter, TriggerDefinition, TypedFilter,
};
use crate::types::card::CardFace;
use crate::types::counter::CounterType;
use crate::types::keywords::Keyword;
use crate::types::triggers::TriggerMode;
use crate::types::zones::Zone;

/// CR 702.38a: Synthesize the Amplify ETB reveal-and-count ability for every
/// `Keyword::Amplify(N)` printed on the face. Idempotent — re-running
/// `synthesize_all` does not stack duplicate triggers.
pub fn synthesize_amplify(face: &mut CardFace) {
    let triggers: Vec<TriggerDefinition> = face
        .keywords
        .iter()
        .filter_map(|kw| match kw {
            Keyword::Amplify(n) => Some(amplify_trigger(*n)),
            _ => None,
        })
        .collect();
    if triggers.is_empty() || face.triggers.iter().any(is_amplify_trigger) {
        return;
    }
    face.triggers.extend(triggers);
}

/// CR 702.38a: the ETB trigger — reveal any number of creature-type-sharing
/// cards from hand, then enter with `N` counters per revealed card.
fn amplify_trigger(n: u32) -> TriggerDefinition {
    // CR 702.38a: "N +1/+1 counters on it for each card revealed this way."
    let put_counters = AbilityDefinition::new(
        AbilityKind::Spell,
        Effect::PutCounter {
            counter_type: CounterType::Plus1Plus1,
            count: amplify_counter_quantity(n),
            target: TargetFilter::SelfRef,
        },
    );
    // CR 702.38a: "reveal any number of cards from your hand that share a
    // creature type with it" — an optional multi-select into the tracked set.
    let reveal = AbilityDefinition::new(
        AbilityKind::Spell,
        Effect::ChooseObjectsIntoTrackedSet {
            chooser: TargetFilter::Controller,
            filter: amplify_reveal_filter(),
            min: 0,
            max: None,
        },
    )
    .sub_ability(put_counters);

    TriggerDefinition::new(TriggerMode::ChangesZone)
        .destination(Zone::Battlefield)
        .valid_card(TargetFilter::SelfRef)
        .execute(reveal)
        .description(format!(
            "CR 702.38a: Amplify {n} — as this enters, reveal any number of cards from your hand \
             that share a creature type with it; it enters with {n} +1/+1 counter(s) for each."
        ))
}

/// CR 702.38a: `N ×` the number of cards revealed (`TrackedSetSize`). For `N = 1`
/// the multiplier is omitted.
fn amplify_counter_quantity(n: u32) -> QuantityExpr {
    let revealed = QuantityExpr::Ref {
        qty: QuantityRef::TrackedSetSize,
    };
    if n == 1 {
        revealed
    } else {
        QuantityExpr::Multiply {
            factor: n as i32,
            inner: Box::new(revealed),
        }
    }
}

/// CR 702.38a: "cards from your hand that share a creature type with it." A card
/// in the controller's hand whose creature types overlap the entering
/// permanent's (`SelfRef`). Changeling — all creature types — satisfies the
/// share on either side via `GameObject::creature_types`.
fn amplify_reveal_filter() -> TargetFilter {
    TargetFilter::Typed(
        TypedFilter::card()
            .controller(ControllerRef::You)
            .properties(vec![
                FilterProp::InZone { zone: Zone::Hand },
                FilterProp::SharesQuality {
                    quality: SharedQuality::CreatureType,
                    reference: Some(Box::new(TargetFilter::SelfRef)),
                    relation: SharedQualityRelation::Shares,
                },
            ]),
    )
}

/// Idempotency probe: a trigger whose execute is the Amplify hand-reveal.
fn is_amplify_trigger(trigger: &TriggerDefinition) -> bool {
    trigger.execute.as_ref().is_some_and(|a| {
        matches!(
            a.effect.as_ref(),
            Effect::ChooseObjectsIntoTrackedSet { .. }
        ) && a
            .sub_ability
            .as_deref()
            .is_some_and(|sub| matches!(sub.effect.as_ref(), Effect::PutCounter { .. }))
    })
}
