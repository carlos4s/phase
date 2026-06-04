//! Tests for Cipher (CR 702.99). Declared from `game/mod.rs` so `cipher.rs`
//! stays implementation-only.

use super::cipher::{
    begin_encode_choice, collect_combat_damage_recast_triggers, encoded_cards_on_creature,
    finish_encode, handle_encode_choice, legal_encode_creatures, spell_can_encode,
};
use super::zones::create_object;
use crate::types::ability::{Effect, TargetFilter};
use crate::types::card_type::CoreType;
use crate::types::events::GameEvent;
use crate::types::game_state::{GameState, WaitingFor};
use crate::types::identifiers::{CardId, ObjectId};
use crate::types::keywords::Keyword;
use crate::types::player::PlayerId;
use crate::types::zones::Zone;

fn creature(state: &mut GameState, card: u64, owner: PlayerId, name: &str, zone: Zone) -> ObjectId {
    let id = create_object(state, CardId(card), owner, name.to_string(), zone);
    let obj = state.objects.get_mut(&id).unwrap();
    obj.card_types.core_types.push(CoreType::Creature);
    obj.base_card_types.core_types.push(CoreType::Creature);
    id
}

/// A Cipher instant on the stack, controlled by `owner`.
fn cipher_spell(state: &mut GameState, card: u64, owner: PlayerId) -> ObjectId {
    let id = create_object(
        state,
        CardId(card),
        owner,
        "Hidden Strings".to_string(),
        Zone::Stack,
    );
    let obj = state.objects.get_mut(&id).unwrap();
    obj.card_types.core_types.push(CoreType::Instant);
    obj.base_card_types.core_types.push(CoreType::Instant);
    obj.keywords.push(Keyword::Cipher);
    id
}

/// CR 702.99a: only creatures the player controls are legal encode hosts.
#[test]
fn legal_encode_creatures_filters_by_controller() {
    let mut state = GameState::new_two_player(1);
    let mine = creature(&mut state, 1, PlayerId(0), "Mine", Zone::Battlefield);
    let _theirs = creature(&mut state, 2, PlayerId(1), "Theirs", Zone::Battlefield);
    let legal = legal_encode_creatures(&state, PlayerId(0));
    assert_eq!(legal, vec![mine]);
}

/// CR 702.99b: finishing the encode exiles the card and records the link.
#[test]
fn finish_encode_exiles_card_and_records_link() {
    let mut state = GameState::new_two_player(1);
    let host = creature(&mut state, 1, PlayerId(0), "Host", Zone::Battlefield);
    let card = create_object(
        &mut state,
        CardId(2),
        PlayerId(0),
        "Spell".to_string(),
        Zone::Stack,
    );

    finish_encode(&mut state, card, host, &mut Vec::new());

    assert_eq!(state.objects[&card].zone, Zone::Exile);
    assert_eq!(encoded_cards_on_creature(&state, host), vec![card]);
}

/// CR 702.99c: the encode link drops when the card leaves exile.
#[test]
fn encode_link_drops_when_card_leaves_exile() {
    let mut state = GameState::new_two_player(1);
    let host = creature(&mut state, 1, PlayerId(0), "Host", Zone::Battlefield);
    let card = create_object(
        &mut state,
        CardId(2),
        PlayerId(0),
        "Spell".to_string(),
        Zone::Stack,
    );
    finish_encode(&mut state, card, host, &mut Vec::new());

    super::zones::move_to_zone(&mut state, card, Zone::Graveyard, &mut Vec::new());
    assert!(encoded_cards_on_creature(&state, host).is_empty());
}

/// CR 702.99c: the encode link drops when the creature leaves the battlefield.
#[test]
fn encode_link_drops_when_creature_leaves_battlefield() {
    let mut state = GameState::new_two_player(1);
    let host = creature(&mut state, 1, PlayerId(0), "Host", Zone::Battlefield);
    let card = create_object(
        &mut state,
        CardId(2),
        PlayerId(0),
        "Spell".to_string(),
        Zone::Stack,
    );
    finish_encode(&mut state, card, host, &mut Vec::new());

    super::zones::move_to_zone(&mut state, host, Zone::Graveyard, &mut Vec::new());
    assert!(encoded_cards_on_creature(&state, host).is_empty());
}

// ── Encode offer (on resolution) ──────────────────────────────────────────

/// CR 702.99a: only an encodable cipher card (non-permanent, not a token) can
/// be encoded.
#[test]
fn spell_can_encode_requires_cipher_nonpermanent_card() {
    let mut state = GameState::new_two_player(1);
    let spell = cipher_spell(&mut state, 1, PlayerId(0));
    assert!(spell_can_encode(&state, spell));

    // A token copy of a cipher spell may not be encoded (no card to exile).
    state.objects.get_mut(&spell).unwrap().is_token = true;
    assert!(!spell_can_encode(&state, spell));
}

/// CR 702.99a: with a legal host, the resolving spell pauses for the encode
/// choice; accepting exiles the card and encodes it on the chosen creature.
#[test]
fn begin_encode_choice_pauses_then_accept_encodes() {
    let mut state = GameState::new_two_player(1);
    let host = creature(&mut state, 1, PlayerId(0), "Host", Zone::Battlefield);
    let spell = cipher_spell(&mut state, 2, PlayerId(0));

    assert!(begin_encode_choice(&mut state, spell, PlayerId(0)));
    match &state.waiting_for {
        WaitingFor::CipherEncodeChoice {
            player,
            card_id,
            creatures,
        } => {
            assert_eq!(*player, PlayerId(0));
            assert_eq!(*card_id, spell);
            assert_eq!(creatures, &vec![host]);
        }
        other => panic!("expected CipherEncodeChoice, got {other:?}"),
    }

    handle_encode_choice(&mut state, spell, Some(host), &mut Vec::new());
    assert_eq!(state.objects[&spell].zone, Zone::Exile);
    assert_eq!(encoded_cards_on_creature(&state, host), vec![spell]);
}

/// CR 702.99a: with no creature to host it, there is no encode offer — the
/// caller routes the card to its graveyard.
#[test]
fn begin_encode_choice_skipped_without_host() {
    let mut state = GameState::new_two_player(1);
    let spell = cipher_spell(&mut state, 1, PlayerId(0));
    assert!(!begin_encode_choice(&mut state, spell, PlayerId(0)));
}

/// CR 608.2n: declining the encode puts the card into its owner's graveyard.
#[test]
fn handle_encode_choice_decline_routes_to_graveyard() {
    let mut state = GameState::new_two_player(1);
    let _host = creature(&mut state, 1, PlayerId(0), "Host", Zone::Battlefield);
    let spell = cipher_spell(&mut state, 2, PlayerId(0));
    assert!(begin_encode_choice(&mut state, spell, PlayerId(0)));

    handle_encode_choice(&mut state, spell, None, &mut Vec::new());
    assert_eq!(state.objects[&spell].zone, Zone::Graveyard);
    assert!(state.exile_links.is_empty());
}

// ── Combat-damage recast ──────────────────────────────────────────────────

/// CR 702.99c: an encoded creature dealing combat damage to a player produces
/// the optional "cast a copy of the encoded card" trigger, targeting the card.
#[test]
fn combat_damage_collects_optional_recast_trigger_for_encoded_card() {
    let mut state = GameState::new_two_player(1);
    let host = creature(&mut state, 1, PlayerId(0), "Host", Zone::Battlefield);
    let card = create_object(
        &mut state,
        CardId(2),
        PlayerId(0),
        "Spell".to_string(),
        Zone::Stack,
    );
    finish_encode(&mut state, card, host, &mut Vec::new());

    let event = GameEvent::CombatDamageDealtToPlayer {
        player_id: PlayerId(1),
        source_amounts: vec![(host, 2)],
        total_damage: 2,
    };
    let mut pending = Vec::new();
    collect_combat_damage_recast_triggers(&state, std::slice::from_ref(&event), &mut pending);

    assert_eq!(pending.len(), 1, "one recast trigger for the encoded card");
    let trig = &pending[0].pending;
    assert_eq!(trig.source_id, host);
    assert_eq!(trig.controller, PlayerId(0));
    assert!(
        trig.ability.optional,
        "the recast is optional (\"you may cast\")"
    );
    match &trig.ability.effect {
        Effect::CastCopyOfCard { target, cost } => {
            assert_eq!(target, &TargetFilter::SpecificObject { id: card });
            assert!(
                cost.is_without_paying_mana(),
                "cast without paying its mana cost"
            );
        }
        other => panic!("expected CastCopyOfCard, got {other:?}"),
    }
}

/// CR 702.99c: a creature with no encoded card produces no recast trigger.
#[test]
fn combat_damage_no_trigger_without_encode() {
    let mut state = GameState::new_two_player(1);
    let host = creature(&mut state, 1, PlayerId(0), "Host", Zone::Battlefield);
    let event = GameEvent::CombatDamageDealtToPlayer {
        player_id: PlayerId(1),
        source_amounts: vec![(host, 3)],
        total_damage: 3,
    };
    let mut pending = Vec::new();
    collect_combat_damage_recast_triggers(&state, std::slice::from_ref(&event), &mut pending);
    assert!(pending.is_empty());
}
