//! CR 702.50: Epic. When an Epic spell resolves, two linked effects begin and
//! last for the rest of the game:
//!
//! * CR 702.50b — its controller can't cast spells (but may still activate
//!   abilities, and effects may still put spell copies onto the stack);
//! * CR 702.50a — at the beginning of each of that player's upkeeps, they copy
//!   the spell except for its epic ability, optionally choosing new targets.
//!
//! This module mirrors Rebound (`game/effects/rebound.rs`): an on-resolution
//! arming hook installs a delayed triggered ability keyed on the controller's
//! upkeep. The differences are that (1) the trigger is **recurring**
//! (`one_shot = false`), so it fires every upkeep for the rest of the game, and
//! (2) its body is an [`Effect::EpicCopy`] carrying a snapshot of the Epic
//! spell's resolved ability — when that body resolves it puts a copy of the
//! spell onto the stack (CR 707.10).
//!
//! The copy keeps the snapshot's declared targets: CR 702.50a's "you may choose
//! new targets" is an optional permission whose default — keeping the original
//! (still-legal) targets — is itself a legal choice.

use crate::game::effects::copy_spell::set_resolved_source_recursive;
use crate::types::ability::{
    DelayedTriggerCondition, Effect, EffectError, EffectKind, ResolvedAbility,
};
use crate::types::events::GameEvent;
use crate::types::game_state::{
    CastingVariant, DelayedTrigger, GameState, StackEntry, StackEntryKind,
};
use crate::types::identifiers::ObjectId;
use crate::types::keywords::Keyword;
use crate::types::phase::Phase;
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

/// CR 702.50b: Whether `player` has resolved an Epic spell they control and is
/// therefore locked out of casting spells for the rest of the game.
pub(crate) fn is_epic_locked(state: &GameState, player: PlayerId) -> bool {
    state.epic_locked_players.contains(&player)
}

/// CR 702.50a-b: On-resolution arming hook for a spell carrying `Keyword::Epic`.
/// Called from `stack.rs::resolve_top` once it has confirmed the resolving
/// object is a non-token spell with `Keyword::Epic`.
///
/// * CR 702.50b — locks `controller` out of casting spells for the rest of the
///   game (the Epic card itself still goes to the graveyard normally).
/// * CR 702.50a — pushes a recurring delayed triggered ability keyed on the
///   controller's upkeep whose body ([`Effect::EpicCopy`]) copies the spell.
///   `source_id` is the resolved Epic card, whose characteristics the copy
///   clones; `spell_ability` is the snapshot the copy resolves.
pub(crate) fn arm_epic(
    state: &mut GameState,
    source_id: ObjectId,
    controller: PlayerId,
    spell_ability: ResolvedAbility,
) {
    // CR 702.50b: the controller can no longer cast spells.
    state.epic_locked_players.insert(controller);

    // CR 702.50a: the recurring upkeep copy. The body carries the spell's
    // resolved-ability snapshot, re-sourced to each copy at resolution time.
    let body = ResolvedAbility::new(
        Effect::EpicCopy {
            spell: Box::new(spell_ability),
        },
        Vec::new(),
        source_id,
        controller,
    );

    state.delayed_triggers.push(DelayedTrigger {
        // CR 702.50a: "at the beginning of each of your upkeeps".
        condition: DelayedTriggerCondition::AtNextPhaseForPlayer {
            phase: Phase::Upkeep,
            player: controller,
        },
        ability: body,
        controller,
        source_id,
        // CR 702.50a: "for the rest of the game" — recurring, never removed.
        one_shot: false,
    });
}

/// CR 702.50a + CR 707.10: Resolve [`Effect::EpicCopy`] — put a copy of the
/// snapshotted Epic spell onto the stack under its controller, excluding the
/// epic ability so the copy does not register a fresh Epic effect.
pub(crate) fn resolve(
    state: &mut GameState,
    ability: &ResolvedAbility,
    events: &mut Vec<GameEvent>,
) -> Result<(), EffectError> {
    let Effect::EpicCopy { spell } = &ability.effect else {
        return Err(EffectError::MissingParam("EpicCopy".to_string()));
    };

    // The resolved Epic card supplies the copy's characteristics. If it has
    // left the game (no last-known object), the copy can't be built — resolve
    // as a no-op rather than fabricating a copy.
    let prototype_id = ability.source_id;
    let Some(source_obj) = state.objects.get(&prototype_id) else {
        events.push(GameEvent::EffectResolved {
            kind: EffectKind::from(&ability.effect),
            source_id: ability.source_id,
        });
        return Ok(());
    };

    let controller = ability.controller;
    let copy_id = ObjectId(state.next_object_id);
    state.next_object_id += 1;

    // CR 707.10 + CR 702.50a: clone the Epic card's characteristics, but strip
    // Epic ("except for its epic ability") so the copy's own resolution does
    // not arm a second Epic effect. The copy is a token spell on the stack.
    let mut copy_obj = source_obj.clone();
    copy_obj.id = copy_id;
    copy_obj.controller = controller;
    copy_obj.zone = Zone::Stack;
    copy_obj.is_token = true;
    copy_obj.additional_cost_payment_count = 0;
    copy_obj.kickers_paid.clear();
    copy_obj.keywords.retain(|k| !matches!(k, Keyword::Epic));
    let card_id = copy_obj.card_id;
    state.objects.insert(copy_id, copy_obj);

    // CR 707.10: the copy resolves the snapshotted ability, re-sourced to the
    // copy so every SelfRef resolves to the copy rather than the original.
    let mut copy_ability = (**spell).clone();
    set_resolved_source_recursive(&mut copy_ability, copy_id);

    state.stack.push_back(StackEntry {
        id: copy_id,
        source_id: copy_id,
        controller,
        kind: StackEntryKind::Spell {
            card_id,
            ability: Some(copy_ability),
            casting_variant: CastingVariant::default(),
            actual_mana_spent: 0,
        },
    });
    events.push(GameEvent::StackPushed { object_id: copy_id });

    // CR 707.10: a copy is put on the stack but not cast — `SpellCopied` (not
    // `SpellCast`) so copy-sensitive triggers fire without cast-only triggers.
    events.push(GameEvent::SpellCopied {
        card_id,
        controller,
        object_id: copy_id,
        original_id: prototype_id,
    });

    events.push(GameEvent::EffectResolved {
        kind: EffectKind::from(&ability.effect),
        source_id: ability.source_id,
    });
    Ok(())
}
