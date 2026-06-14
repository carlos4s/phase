//! CR 904: Archenemy — the scheme deck, setting schemes in motion, and the
//! abandon state-based action.
//!
//! Schemes (CR 314) are nontraditional cards that remain in the command zone
//! throughout the game (CR 314.2), both while face down in the scheme deck and
//! while face up after being set in motion. They are not permanents and can't
//! be cast (CR 314.2).
//!
//! In the single-scheme-deck Archenemy option (CR 904.3 / CR 904.4) the engine
//! tracks one deck in [`GameState::scheme_deck`] (front = top, face down).
//! Schemes that are set in motion (CR 904.9) turn face up and live in
//! [`GameState::command_zone`]; non-ongoing schemes are abandoned back to the
//! bottom of the scheme deck by a state-based action (CR 904.10 / CR 314.6),
//! while ongoing schemes (CR 904.11) stay face up until an ability abandons
//! them.
//!
//! This is the runtime sibling of `game::planechase`: it owns setting a scheme
//! in motion ([`set_in_motion`]), abandoning a scheme ([`abandon`]), and the
//! abandon state-based action ([`check_scheme_abandon_sba`]).
//!
//! Trigger collection mirrors planechase: scheme triggers function from the
//! command zone because `synthesize_archenemy` stamps
//! `trigger_zones = [Zone::Command]` onto them (CR 113.6b / CR 314.4 /
//! CR 904.8). The set-in-motion and abandon turn-based/state-based actions don't
//! use the stack (CR 904.9 / CR 904.10), but the resulting triggered abilities
//! are deferred to the next priority via `collect_triggers_into_deferred`
//! (CR 603.3).
//!
//! The two-scheme-deck Supervillain Rumble option (CR 904.12) is deferred: this
//! module models the single-archenemy game (CR 904.2a).

use crate::types::card_type::{CoreType, Supertype};
use crate::types::events::GameEvent;
use crate::types::game_state::GameState;
use crate::types::identifiers::ObjectId;

/// CR 314: True when the object is a scheme card.
pub fn is_scheme_object(state: &GameState, id: ObjectId) -> bool {
    state
        .objects
        .get(&id)
        .is_some_and(|o| o.card_types.core_types.contains(&CoreType::Scheme))
}

/// CR 904.9: The top card of the archenemy's scheme deck (front = top), or
/// `None` if the scheme deck is empty.
pub fn top_scheme(state: &GameState) -> Option<ObjectId> {
    state.scheme_deck.front().copied()
}

/// CR 904.9 / CR 904.11: The schemes currently set in motion — command-zone
/// scheme cards that are face up. Returns a `Vec` because ongoing schemes
/// (CR 904.11) accumulate: more than one scheme can be face up at once, unlike
/// Planechase's single active plane.
pub fn active_schemes(state: &GameState) -> Vec<ObjectId> {
    state
        .command_zone
        .iter()
        .copied()
        .filter(|&id| {
            is_scheme_object(state, id) && state.objects.get(&id).is_some_and(|o| !o.face_down)
        })
        .collect()
}

/// CR 904.9 / CR 701.32b: Set the top scheme of the archenemy's scheme deck in
/// motion — move it off the top of the scheme deck and turn it face up in the
/// command zone.
///
/// No-op outside an Archenemy game (no archenemy designated) or when the scheme
/// deck is empty. Otherwise the top scheme is popped from `scheme_deck`, turned
/// face up, stamped with the archenemy as its controller (CR 314.5: the
/// controller of a face-up scheme is its owner — the archenemy — so its
/// "you"-scoped SetInMotion trigger resolves for the archenemy), and pushed
/// into the command zone.
///
/// CR 603.3: the `SchemeSetInMotion` event triggers the scheme's "When you set
/// this scheme in motion" ability (`SetInMotion`). The turn-based action itself
/// doesn't use the stack (CR 904.9), but the resulting trigger is deferred to
/// the next priority.
pub fn set_in_motion(state: &mut GameState, events: &mut Vec<GameEvent>) {
    let Some(archenemy) = state.archenemy else {
        return;
    };
    // CR 701.32b: move the scheme off the top of the scheme deck.
    let Some(scheme_id) = state.scheme_deck.pop_front() else {
        return;
    };
    // CR 701.32b / CR 314.5: turn it face up and stamp the archenemy as its
    // controller so its "you"-scoped triggers resolve for the archenemy.
    if let Some(obj) = state.objects.get_mut(&scheme_id) {
        obj.face_down = false;
        obj.controller = archenemy;
    }
    // CR 314.2 / CR 904.9: the scheme stays in the command zone, now face up.
    state.command_zone.push_back(scheme_id);

    // CR 904.9 / CR 701.32b: announce that the scheme was set in motion.
    let event = GameEvent::SchemeSetInMotion {
        player_id: archenemy,
        scheme_id,
    };
    events.push(event.clone());

    // CR 603.3: defer the SetInMotion trigger to the next priority.
    crate::game::triggers::collect_triggers_into_deferred(state, &[event]);
}

/// CR 904.10 / CR 314.6: True if any scheme's triggered ability is on the stack
/// or waiting to be put on the stack (i.e. deferred but not yet on the stack).
/// While any such ability exists, the abandon state-based action does nothing.
pub fn scheme_trigger_on_stack_or_pending(state: &GameState) -> bool {
    // CR 904.10: "on the stack" — any stack entry sourced from a scheme.
    let on_stack = state
        .stack
        .iter()
        .any(|entry| is_scheme_object(state, entry.source_id));
    // CR 904.10: "waiting to be put on the stack" — any deferred trigger whose
    // pending source is a scheme.
    let pending = state
        .deferred_triggers
        .iter()
        .any(|d| is_scheme_object(state, d.pending.source_id));
    on_stack || pending
}

/// CR 701.33b / CR 904.10: Abandon a scheme — turn it face down and put it on
/// the bottom of its owner's scheme deck.
///
/// CR 603.3: the `SchemeAbandoned` event triggers the scheme's "When you
/// abandon this scheme" ability (`Abandoned`), deferred to the next priority.
pub fn abandon(state: &mut GameState, scheme_id: ObjectId, events: &mut Vec<GameEvent>) {
    // CR 904.7 / CR 314.5: the owner/controller of a scheme is the archenemy.
    let owner = state
        .archenemy
        .or_else(|| state.objects.get(&scheme_id).map(|o| o.controller));

    // CR 701.33b / CR 904.10: announce that the scheme was abandoned.
    let event = GameEvent::SchemeAbandoned {
        player_id: owner.unwrap_or(state.active_player),
        scheme_id,
    };
    events.push(event.clone());

    // CR 314.4 / CR 904.8 + CR 603.3: a scheme's triggered abilities may trigger
    // only while it is face up in the command zone, so collect its "when you
    // abandon this scheme" trigger while the scheme is still face up — BEFORE the
    // face-down flip below — and defer it to the next priority. (Unlike
    // `planechase::planeswalk`, which must flip the departing plane face down
    // first so the arriving face-up plane can share a single trigger scan, abandon
    // touches one scheme, so it collects the trigger while that scheme is face up.)
    crate::game::triggers::collect_triggers_into_deferred(state, &[event]);

    // CR 701.33b / CR 314.2: now turn the scheme face down and put it on the
    // bottom of its owner's scheme deck (front = top), removing it from the
    // active command-zone view.
    if let Some(obj) = state.objects.get_mut(&scheme_id) {
        obj.face_down = true;
    }
    state.command_zone.retain(|&id| id != scheme_id);
    state.scheme_deck.push_back(scheme_id);
}

/// CR 904.10 / CR 314.6: State-based action — a face-up non-ongoing scheme card
/// in the command zone, with no scheme triggered ability on the stack or
/// waiting to be put on the stack, is turned face down and put on the bottom of
/// its owner's scheme deck.
///
/// Gated on an Archenemy game (`archenemy.is_some()`). Ongoing schemes
/// (CR 904.11) are exempt — they stay face up until an ability abandons them.
/// Mirrors `planechase::check_phenomenon_planeswalk_sba`: records that an action
/// was performed so the SBA fixpoint loop re-checks.
pub fn check_scheme_abandon_sba(
    state: &mut GameState,
    events: &mut Vec<GameEvent>,
    any_performed: &mut bool,
) {
    if state.archenemy.is_none() {
        return;
    }
    // CR 904.10: do nothing while any scheme's triggered ability is on the stack
    // or waiting to be put on the stack.
    if scheme_trigger_on_stack_or_pending(state) {
        return;
    }
    // CR 904.10 / CR 904.11: abandon every face-up non-ongoing scheme; ongoing
    // schemes are exempt. Collect first to avoid borrowing `state` while mutating.
    let to_abandon: Vec<ObjectId> = active_schemes(state)
        .into_iter()
        .filter(|&id| {
            state
                .objects
                .get(&id)
                .is_some_and(|o| !o.card_types.supertypes.contains(&Supertype::Ongoing))
        })
        .collect();
    for scheme_id in to_abandon {
        abandon(state, scheme_id, events);
        *any_performed = true;
    }
}
