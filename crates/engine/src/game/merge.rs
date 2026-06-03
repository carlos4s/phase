//! CR 730 (Merging with Permanents) + CR 702.140 (Mutate).
//!
//! Phase 1 of the Mutate keyword. A mutating creature spell that resolves with a
//! legal target does NOT enter the battlefield (CR 702.140c); instead it merges
//! with the target creature, becoming one object represented by more than one
//! card or token (CR 730.2). This module owns the merge primitive
//! ([`merge_object_onto`]), the leave-the-battlefield split (CR 730.3,
//! [`split_merged_permanent_on_leave`]), and the top/bottom choice handler
//! ([`handle_mutate_merge_choice`]).
//!
//! Merge identity (BINDING review resolution S4):
//!   * The surviving battlefield object is ALWAYS the target creature's
//!     `ObjectId` (CR 730.2c continuity). The merged permanent "is the same
//!     object that it was before."
//!   * Over/under only selects which component supplies copiable characteristics
//!     (CR 730.2a) — recorded as the topmost element of
//!     `GameObject::merged_components` (convention: index `[0]` is topmost).
//!   * The merged permanent always has the UNION of every component's abilities
//!     (CR 702.140e); its other characteristics come from the topmost component
//!     (CR 730.2a).
//!   * Each component retains its ORIGINAL owner so the CR 730.3 leave-split
//!     routes each card/token to the correct player's zone.
//!
//! Multi-instance stacking (CR 730.2) IS supported: mutating onto an
//! already-merged permanent extends its component stack, and the merged
//! permanent's identity is re-derived from the full stack each time. The
//! survivor's intrinsic identity is preserved in `GameObject::merge_self_origin`
//! so it reverts to its own card on leaving the battlefield (CR 730.3 + 400.7).
//!
//! Deferred: copy effects targeting a merged permanent, face-down/DFC
//! components, full CR 702.140d downstream reflexive effects, and the CR 730.3a
//! graveyard/library arrange-order UI (a deterministic order is used).

use crate::game::game_object::{GameObject, MergeSelfOrigin};
use crate::types::card::PrintedCardRef;
use crate::types::card_type::CardType;
use crate::types::events::GameEvent;
use crate::types::game_state::GameState;
use crate::types::identifiers::ObjectId;
use crate::types::mana::{ManaColor, ManaCost};
use crate::types::zones::Zone;

/// CR 702.140c + CR 730.2a: Which side of the target creature the mutating
/// spell is placed on. The choice selects the topmost component (copiable
/// characteristics, CR 730.2a); it never changes the merged permanent's
/// `ObjectId` (CR 730.2c). A typed enum rather than a `bool` so call sites are
/// self-documenting and exhaustively matched.
///
/// Serializes as the plain variant string ("Top" / "Bottom") so the frontend
/// `GameAction::ChooseMutateMergeSide` payload is `{ side: "Top" | "Bottom" }`,
/// parallel to the sibling `ChooseTopOrBottom { top: bool }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MergeSide {
    /// The mutating spell is placed on TOP of the target creature — the spell's
    /// card/token supplies the copiable characteristics.
    Top,
    /// The mutating spell is placed UNDER the target creature — the target keeps
    /// its own copiable characteristics.
    Bottom,
}

/// CR 702.140c + CR 730.2: Merge `merging_id` (the resolving mutate spell's
/// card/token) onto `target_id` (the surviving battlefield creature) on the
/// chosen `side`.
///
/// The target keeps its `ObjectId` (CR 730.2c); `side` only sets the topmost
/// component. The merged permanent gains the union of all components' abilities
/// (CR 702.140e) and the topmost component's other characteristics (CR 730.2a,
/// snapshot into both live and base fields so a layer re-evaluation preserves
/// them, with `layers_dirty` marked). The permanent is NOT considered to have
/// entered the battlefield (CR 730.2b/c), so no ETB triggers fire. Emits
/// `GameEvent::Mutated`.
///
/// CR 730.2 multi-instance stacking: if `target_id` is already a merged
/// permanent, `merging_id` extends its component stack (over or under the whole
/// stack per `side`); the identity is re-derived from the full stack. The
/// survivor's intrinsic identity is captured into `merge_self_origin` on the
/// first merge so the union/copiable can be recomputed from scratch and the
/// survivor reverts to its own card on leaving the battlefield. `merging_id`'s
/// `GameObject` is retained in `state.objects` as a component (it has left the
/// stack in `stack::resolve_top`) so [`split_merged_permanent_on_leave`] can
/// restore it.
pub fn merge_object_onto(
    state: &mut GameState,
    merging_id: ObjectId,
    target_id: ObjectId,
    side: MergeSide,
    events: &mut Vec<GameEvent>,
) {
    // Resolve the merging spell's controller for the event payload before any
    // mutation (the component object survives, so this stays valid).
    let controller = state
        .objects
        .get(&merging_id)
        .map(|o| o.controller)
        .or_else(|| state.objects.get(&target_id).map(|o| o.controller))
        .expect("merge components exist");

    // CR 730.2b/c: the merging card leaves the stack and becomes part of the
    // battlefield object identified by `target_id`. It is not itself in any zone
    // list; mark its zone as Battlefield so component queries see a consistent
    // location. The stack entry was already popped in `stack::resolve_top`. The
    // stack-only `mutate_form` marker is cleared — it is now a component.
    if let Some(merging) = state.objects.get_mut(&merging_id) {
        merging.zone = Zone::Battlefield;
        merging.mutate_form = None;
    }

    // CR 730.2 + 702.140e: capture the survivor's intrinsic identity the FIRST
    // time it merges (its live/base fields will be overwritten with the derived
    // merged form below). Idempotent — a re-merge keeps the original snapshot.
    capture_self_origin_if_needed(state, target_id);

    // CR 730.2 multi-instance stacking: extend the existing stack when
    // `target_id` is already merged; otherwise start from the survivor itself.
    // Convention: element [0] is the topmost component (CR 730.2a).
    let existing: Vec<ObjectId> = state
        .objects
        .get(&target_id)
        .map(|o| o.merged_components.clone())
        .unwrap_or_default();
    let base_order = if existing.is_empty() {
        vec![target_id]
    } else {
        existing
    };
    let ordered: Vec<ObjectId> = match side {
        MergeSide::Top => {
            let mut v = Vec::with_capacity(base_order.len() + 1);
            v.push(merging_id);
            v.extend(base_order);
            v
        }
        MergeSide::Bottom => {
            let mut v = base_order;
            v.push(merging_id);
            v
        }
    };
    let topmost_id = ordered[0];

    // CR 730.2a: the topmost component supplies the copiable characteristics.
    // CR 702.140e: the merged permanent has the UNION of every component's
    // abilities. Both re-derive from each component's INTRINSIC identity (the
    // survivor's intrinsic comes from its snapshot, since its own fields now hold
    // the previously-derived merged form).
    apply_topmost_copiable(state, topmost_id, target_id);
    union_abilities_onto(state, &ordered, target_id);

    if let Some(survivor) = state.objects.get_mut(&target_id) {
        survivor.merged_components = ordered;
    }

    // CR 613.2: the merge is a copiable effect; re-run layers so any continuous
    // effects re-derive against the merged characteristics.
    state.layers_dirty.mark_full();

    // CR 702.140c-d: the mutation is observable. NO ETB (CR 730.2b/c).
    events.push(GameEvent::Mutated {
        merged_id: target_id,
        merging_id,
        controller,
    });
}

/// CR 730.2 + 702.140e: Capture the survivor's intrinsic characteristics into
/// `merge_self_origin` the first time it becomes a merged permanent. No-op if it
/// is already merged (the snapshot already holds the un-merged identity).
fn capture_self_origin_if_needed(state: &mut GameState, target_id: ObjectId) {
    let Some(obj) = state.objects.get(&target_id) else {
        return;
    };
    if obj.merge_self_origin.is_some() {
        return;
    }
    let origin = MergeSelfOrigin {
        name: obj.name.clone(),
        base_name: obj.base_name.clone(),
        mana_cost: obj.mana_cost.clone(),
        base_mana_cost: obj.base_mana_cost.clone(),
        color: obj.color.clone(),
        base_color: obj.base_color.clone(),
        card_types: obj.card_types.clone(),
        base_card_types: obj.base_card_types.clone(),
        power: obj.power,
        toughness: obj.toughness,
        base_power: obj.base_power,
        base_toughness: obj.base_toughness,
        loyalty: obj.loyalty,
        base_loyalty: obj.base_loyalty,
        printed_ref: obj.printed_ref.clone(),
        base_printed_ref: obj.base_printed_ref.clone(),
        abilities: obj.abilities.clone(),
        base_abilities: obj.base_abilities.clone(),
        trigger_definitions: obj.trigger_definitions.clone(),
        base_trigger_definitions: obj.base_trigger_definitions.clone(),
        static_definitions: obj.static_definitions.clone(),
        base_static_definitions: obj.base_static_definitions.clone(),
        replacement_definitions: obj.replacement_definitions.clone(),
        base_replacement_definitions: obj.base_replacement_definitions.clone(),
        keywords: obj.keywords.clone(),
        base_keywords: obj.base_keywords.clone(),
    };
    if let Some(obj) = state.objects.get_mut(&target_id) {
        obj.merge_self_origin = Some(Box::new(origin));
    }
}

/// CR 707.2 copiable characteristics carried by the topmost merge component.
struct CopiableForm {
    name: String,
    base_name: String,
    mana_cost: ManaCost,
    base_mana_cost: ManaCost,
    color: Vec<ManaColor>,
    base_color: Vec<ManaColor>,
    card_types: CardType,
    base_card_types: CardType,
    power: Option<i32>,
    toughness: Option<i32>,
    base_power: Option<i32>,
    base_toughness: Option<i32>,
    loyalty: Option<u32>,
    base_loyalty: Option<u32>,
    printed_ref: Option<PrintedCardRef>,
    base_printed_ref: Option<PrintedCardRef>,
}

impl CopiableForm {
    fn from_object(o: &GameObject) -> Self {
        Self {
            name: o.name.clone(),
            base_name: o.base_name.clone(),
            mana_cost: o.mana_cost.clone(),
            base_mana_cost: o.base_mana_cost.clone(),
            color: o.color.clone(),
            base_color: o.base_color.clone(),
            card_types: o.card_types.clone(),
            base_card_types: o.base_card_types.clone(),
            power: o.power,
            toughness: o.toughness,
            base_power: o.base_power,
            base_toughness: o.base_toughness,
            loyalty: o.loyalty,
            base_loyalty: o.base_loyalty,
            printed_ref: o.printed_ref.clone(),
            base_printed_ref: o.base_printed_ref.clone(),
        }
    }

    fn from_origin(o: &MergeSelfOrigin) -> Self {
        Self {
            name: o.name.clone(),
            base_name: o.base_name.clone(),
            mana_cost: o.mana_cost.clone(),
            base_mana_cost: o.base_mana_cost.clone(),
            color: o.color.clone(),
            base_color: o.base_color.clone(),
            card_types: o.card_types.clone(),
            base_card_types: o.base_card_types.clone(),
            power: o.power,
            toughness: o.toughness,
            base_power: o.base_power,
            base_toughness: o.base_toughness,
            loyalty: o.loyalty,
            base_loyalty: o.base_loyalty,
            printed_ref: o.printed_ref.clone(),
            base_printed_ref: o.base_printed_ref.clone(),
        }
    }

    fn write_to(self, dst: &mut GameObject) {
        dst.name = self.name;
        dst.base_name = self.base_name;
        dst.mana_cost = self.mana_cost;
        dst.base_mana_cost = self.base_mana_cost;
        dst.color = self.color;
        dst.base_color = self.base_color;
        dst.card_types = self.card_types;
        dst.base_card_types = self.base_card_types;
        dst.power = self.power;
        dst.toughness = self.toughness;
        dst.base_power = self.base_power;
        dst.base_toughness = self.base_toughness;
        dst.loyalty = self.loyalty;
        dst.base_loyalty = self.base_loyalty;
        // Display identity follows the topmost component (CR 730.2a).
        dst.printed_ref = self.printed_ref;
        dst.base_printed_ref = self.base_printed_ref;
        // NOTE: keywords are NOT copied here — they are keyword ABILITIES that
        // belong to the CR 702.140e union (`union_abilities_onto`), which reads
        // each component's intrinsic keyword set.
    }
}

/// CR 730.2a: Apply the topmost component's intrinsic copiable characteristics
/// onto the surviving object. When the survivor itself is topmost (e.g. a Bottom
/// merge), its intrinsic identity is read from its `merge_self_origin` snapshot —
/// its own fields hold the previously-derived merged form. Writes both live and
/// layer-7b base fields so the merged form survives a layer re-evaluation
/// (which anchors on base values).
fn apply_topmost_copiable(state: &mut GameState, topmost_id: ObjectId, target_id: ObjectId) {
    let form = if topmost_id == target_id {
        state
            .objects
            .get(&target_id)
            .and_then(|o| o.merge_self_origin.as_deref())
            .map(CopiableForm::from_origin)
    } else {
        state
            .objects
            .get(&topmost_id)
            .map(CopiableForm::from_object)
    };
    let Some(form) = form else {
        return;
    };
    if let Some(dst) = state.objects.get_mut(&target_id) {
        form.write_to(dst);
    }
}

/// CR 702.140e: A mutated permanent has all abilities of every card/token that
/// represents it. Union each component's INTRINSIC base ability set (abilities,
/// triggers, statics, replacements, keywords) onto the surviving object's base
/// fields, then mirror onto the live fields so the union is visible before the
/// next layer pass. The survivor's intrinsic contribution comes from its
/// `merge_self_origin` snapshot (its own base fields now hold the derived union);
/// every other component's base fields are intact. Components are read in
/// `ordered` (topmost-first).
fn union_abilities_onto(state: &mut GameState, ordered: &[ObjectId], target_id: ObjectId) {
    use std::sync::Arc;

    let mut abilities = Vec::new();
    let mut triggers = Vec::new();
    let mut statics = Vec::new();
    let mut replacements = Vec::new();
    let mut keywords: Vec<crate::types::keywords::Keyword> = Vec::new();

    type BaseSets = (
        Arc<Vec<crate::types::ability::AbilityDefinition>>,
        Arc<Vec<crate::types::ability::TriggerDefinition>>,
        Arc<Vec<crate::types::ability::StaticDefinition>>,
        Arc<Vec<crate::types::ability::ReplacementDefinition>>,
        Vec<crate::types::keywords::Keyword>,
    );

    for &component_id in ordered {
        // CR 613.1: read intrinsic (base) sets so the union is independent of
        // transient layer effects. For the survivor, its intrinsic lives in the
        // snapshot; for every other component, in its own base fields. Clone the
        // (cheap, `Arc`-shared) sets out first so the accumulators aren't borrowed
        // against `state`.
        let sets: Option<BaseSets> = if component_id == target_id {
            state
                .objects
                .get(&target_id)
                .and_then(|o| o.merge_self_origin.as_deref())
                .map(|origin| {
                    (
                        origin.base_abilities.clone(),
                        origin.base_trigger_definitions.clone(),
                        origin.base_static_definitions.clone(),
                        origin.base_replacement_definitions.clone(),
                        origin.base_keywords.clone(),
                    )
                })
        } else {
            state.objects.get(&component_id).map(|obj| {
                (
                    obj.base_abilities.clone(),
                    obj.base_trigger_definitions.clone(),
                    obj.base_static_definitions.clone(),
                    obj.base_replacement_definitions.clone(),
                    obj.base_keywords.clone(),
                )
            })
        };
        let Some((abil, trig, stat, repl, kws)) = sets else {
            continue;
        };
        abilities.extend(abil.iter().cloned());
        triggers.extend(trig.iter().cloned());
        statics.extend(stat.iter().cloned());
        replacements.extend(repl.iter().cloned());
        for kw in kws {
            if !keywords.contains(&kw) {
                keywords.push(kw);
            }
        }
    }

    let Some(dst) = state.objects.get_mut(&target_id) else {
        return;
    };
    dst.base_abilities = Arc::new(abilities.clone());
    dst.base_trigger_definitions = Arc::new(triggers.clone());
    dst.base_static_definitions = Arc::new(statics.clone());
    dst.base_replacement_definitions = Arc::new(replacements.clone());
    dst.base_keywords = keywords.clone();
    // Mirror onto the live fields; layer evaluation will rebuild from base, but
    // any read before the next flush sees the union.
    dst.abilities = Arc::new(abilities);
    dst.trigger_definitions = triggers.into();
    dst.static_definitions = statics.into();
    dst.replacement_definitions = replacements.into();
    dst.keywords = keywords;
}

/// CR 730.3: When a merged permanent leaves the battlefield, one permanent
/// leaves and EACH component is put into the appropriate zone. Each component
/// goes to its OWN owner's `dest` zone (S4: components retain their original
/// owner). The surviving object (`merged_id`) is moved by the normal
/// `move_to_zone` flow; this routes the OTHER components.
///
/// Called from the battlefield-exit seam in `zones::move_to_zone` BEFORE the
/// surviving object is moved. Returns immediately for non-merged objects.
///
/// CR 730.3a deferred: the owner's arrange-order choice for graveyard/library
/// destinations is not modeled — components are placed in their stored
/// (topmost-first) order.
pub fn split_merged_permanent_on_leave(
    state: &mut GameState,
    merged_id: ObjectId,
    dest: Zone,
    events: &mut Vec<GameEvent>,
) {
    let Some(survivor) = state.objects.get(&merged_id) else {
        return;
    };
    if survivor.merged_components.is_empty() {
        return;
    }
    let components = survivor.merged_components.clone();

    for component_id in components {
        // The surviving object itself rides the normal `move_to_zone` flow; only
        // the absorbed (non-survivor) components need explicit routing here.
        if component_id == merged_id {
            continue;
        }
        // CR 730.3 + S4: route each component to ITS OWN owner's destination zone.
        crate::game::zones::move_to_zone(state, component_id, dest, events);
    }

    // The surviving object's merge identity is cleared by its own
    // `reset_for_battlefield_exit` during the subsequent `move_to_zone`.
}

/// CR 702.140c + CR 730.2a: Resolve the controller's top/bottom choice for a
/// paused mutating creature spell. Consumes `state.pending_mutate_merge`, performs
/// the merge, and returns the engine to priority. Errors if no merge is pending or
/// the acting player is not the spell's controller.
pub fn handle_mutate_merge_choice(
    state: &mut GameState,
    player: crate::types::player::PlayerId,
    side: MergeSide,
    events: &mut Vec<GameEvent>,
) -> Result<crate::types::game_state::WaitingFor, crate::game::engine::EngineError> {
    use crate::game::engine::EngineError;

    let pending = state
        .pending_mutate_merge
        .take()
        .ok_or_else(|| EngineError::ActionNotAllowed("No mutate merge is pending".to_string()))?;
    if pending.controller != player {
        // Restore the pending state so the correct player can still act.
        state.pending_mutate_merge = Some(pending);
        return Err(EngineError::ActionNotAllowed(
            "Only the mutate spell's controller may choose the merge side".to_string(),
        ));
    }

    merge_object_onto(state, pending.merging_id, pending.target_id, side, events);

    // CR 702.140c: resolution is complete; hand priority back to the active
    // player so SBAs/triggers from the `Mutated` event can be processed.
    Ok(crate::types::game_state::WaitingFor::Priority {
        player: state.active_player,
    })
}
