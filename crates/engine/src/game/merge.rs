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
//! Deferred (Phase 1): merging onto an already-merged permanent / multi-instance
//! stacking, copy effects, face-down/DFC components, full CR 702.140d downstream
//! reflexive effects, and the CR 730.3a graveyard/library arrange-order UI (a
//! deterministic order is used).

use crate::types::events::GameEvent;
use crate::types::game_state::GameState;
use crate::types::identifiers::ObjectId;
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
/// Phase 1 precondition: `target_id` is not already a merged permanent
/// (multi-instance stacking is deferred). `merging_id`'s `GameObject` is retained
/// in `state.objects` as a component (it has left the stack in
/// `stack::resolve_top`) so [`split_merged_permanent_on_leave`] can restore it.
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
    // location. The stack entry was already popped in `stack::resolve_top`.
    if let Some(merging) = state.objects.get_mut(&merging_id) {
        merging.zone = Zone::Battlefield;
    }

    // CR 730.2a: snapshot the topmost component's copiable characteristics onto
    // the surviving object. When the mutating spell is on TOP, the survivor adopts
    // the spell's name/P-T/types/etc.; on BOTTOM it keeps its own (a no-op copy).
    // CR 702.140e: union every component's abilities regardless of side.
    let topmost_id = match side {
        MergeSide::Top => merging_id,
        MergeSide::Bottom => target_id,
    };

    // Component order convention: index [0] is the topmost component.
    let ordered: Vec<ObjectId> = match side {
        MergeSide::Top => vec![merging_id, target_id],
        MergeSide::Bottom => vec![target_id, merging_id],
    };

    if topmost_id != target_id {
        copy_copiable_characteristics(state, topmost_id, target_id);
    }
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

/// CR 730.2a: Copy the topmost component's copiable characteristics onto the
/// surviving object. Writes both the live fields and the layer-7b base fields so
/// the merged form survives a layer re-evaluation (which anchors on base values),
/// mirroring `apply_bestow_aura_form`'s dual-field write.
fn copy_copiable_characteristics(state: &mut GameState, from_id: ObjectId, to_id: ObjectId) {
    // CR 707.2: copiable values — name, mana cost, color, card types, P/T,
    // loyalty/defense, abilities (handled via the union), and keywords. Clone out
    // of the source first to avoid an aliasing borrow of `state.objects`.
    let Some(src) = state.objects.get(&from_id) else {
        return;
    };
    let name = src.name.clone();
    let base_name = src.base_name.clone();
    let mana_cost = src.mana_cost.clone();
    let base_mana_cost = src.base_mana_cost.clone();
    let color = src.color.clone();
    let base_color = src.base_color.clone();
    let card_types = src.card_types.clone();
    let base_card_types = src.base_card_types.clone();
    let power = src.power;
    let toughness = src.toughness;
    let base_power = src.base_power;
    let base_toughness = src.base_toughness;
    let loyalty = src.loyalty;
    let base_loyalty = src.base_loyalty;
    let printed_ref = src.printed_ref.clone();
    let base_printed_ref = src.base_printed_ref.clone();

    let Some(dst) = state.objects.get_mut(&to_id) else {
        return;
    };
    dst.name = name;
    dst.base_name = base_name;
    dst.mana_cost = mana_cost;
    dst.base_mana_cost = base_mana_cost;
    dst.color = color;
    dst.base_color = base_color;
    dst.card_types = card_types;
    dst.base_card_types = base_card_types;
    dst.power = power;
    dst.toughness = toughness;
    dst.base_power = base_power;
    dst.base_toughness = base_toughness;
    dst.loyalty = loyalty;
    dst.base_loyalty = base_loyalty;
    // Display identity follows the topmost component (CR 730.2a).
    dst.printed_ref = printed_ref;
    dst.base_printed_ref = base_printed_ref;
    // NOTE: keywords are deliberately NOT copied here — they are keyword
    // ABILITIES and belong to the CR 702.140e union (handled by
    // `union_abilities_onto`, which reads each component's intact `base_keywords`).
    // Copying them here would clobber the non-topmost component's keywords before
    // the union runs.
}

/// CR 702.140e: A mutated permanent has all abilities of each card and token that
/// represents it. Union every component's BASE ability set (abilities, triggers,
/// statics, replacements, keywords) onto the surviving object's base fields, then
/// mirror onto the live fields so the union is visible before the next layer
/// pass. Components are read in `ordered` (topmost-first); the surviving object's
/// own contribution comes from whichever element equals `target_id`.
fn union_abilities_onto(state: &mut GameState, ordered: &[ObjectId], target_id: ObjectId) {
    use std::sync::Arc;

    // Collect each component's base ability classes. Reading base sets (CR 613.1)
    // rather than live sets keeps the union independent of transient layer effects.
    let mut abilities = Vec::new();
    let mut triggers = Vec::new();
    let mut statics = Vec::new();
    let mut replacements = Vec::new();
    let mut keywords = Vec::new();

    for &component_id in ordered {
        let Some(obj) = state.objects.get(&component_id) else {
            continue;
        };
        abilities.extend(obj.base_abilities.iter().cloned());
        triggers.extend(obj.base_trigger_definitions.iter().cloned());
        statics.extend(obj.base_static_definitions.iter().cloned());
        replacements.extend(obj.base_replacement_definitions.iter().cloned());
        for kw in &obj.base_keywords {
            if !keywords.contains(kw) {
                keywords.push(kw.clone());
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
