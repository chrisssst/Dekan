use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArmKey {
    pub(crate) champ_id: u32,

    pub(crate) entry_id: Option<u32>,

    pub(crate) mods: u64,

    pub(crate) classic_slot: Option<u32>,

    pub(crate) party: u64,

    pub(crate) second: Option<(u32, u32)>,

    pub(crate) lobby: bool,
}

impl ArmKey {
    pub(crate) fn covers(&self, champ_id: u32) -> bool {
        self.champ_id == champ_id || self.second.is_some_and(|(second, _)| second == champ_id)
    }

    pub(crate) fn entry_for(&self, champ_id: u32) -> Option<u32> {
        if champ_id == self.champ_id {
            self.entry_id
        } else {
            self.second
                .filter(|(second, _)| *second == champ_id)
                .map(|(_, entry_id)| entry_id)
        }
    }

    pub(crate) fn same_own_pick(&self, other: &ArmKey) -> bool {
        ArmKey {
            party: other.party,
            ..*self
        } == *other
    }

    pub(crate) fn picks(&self) -> Vec<(u32, u32)> {
        self.entry_id
            .map(|entry_id| (self.champ_id, entry_id))
            .into_iter()
            .chain(self.second)
            .collect()
    }
}

pub(crate) fn lobby_mods_fingerprint(
    mods: &dekan_core::mods::ModSelection,
    champions: &[u32],
) -> u64 {
    champions.iter().fold(0, |hash, champion| {
        hash.rotate_left(7) ^ mods.fingerprint(Some(*champion))
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArmRequest {
    pub(crate) key: ArmKey,
    pub(crate) due: tokio::time::Instant,
}

impl ArmRequest {
    pub(crate) fn key(&self) -> ArmKey {
        self.key
    }
}

pub(crate) fn is_classic(champ_id: u32) -> bool {
    dekan_classic::builder::is_classic_champion(champ_id)
}

pub(crate) fn build_key(
    champ_id: u32,
    entry_id: Option<u32>,
    mods: &dekan_core::mods::ModSelection,
    client_skin: Option<u32>,
    party: u64,
) -> Option<ArmKey> {
    let classic = is_classic(champ_id);
    let entry_id = if classic {
        entry_id.filter(|e| dekan_classic::generator::skin_number(*e) != 0)
    } else {
        entry_id
    };
    let mods = if classic {
        0
    } else {
        mods.fingerprint(Some(champ_id))
    };
    let party = if classic { 0 } else { party };
    if entry_id.is_none() && mods == 0 && party == 0 {
        return None;
    }
    let classic_slot = if classic {
        client_skin
            .map(dekan_classic::generator::skin_number)
            .filter(|n| !dekan_classic::builder::CLASSIC_DEFAULT_SLOTS.contains(n))
    } else {
        None
    };
    Some(ArmKey {
        champ_id,
        entry_id,
        mods,
        classic_slot,
        party,
        second: None,
        lobby: false,
    })
}

pub(crate) fn arms_in_lobby(state: &dekan_core::state::AppState) -> bool {
    state.lobby.is_some() && state.phase.is_before_champ_select()
}

pub(crate) fn lobby_arm_key(state: &dekan_core::state::AppState) -> Option<ArmKey> {
    let lobby = state.lobby.as_ref()?;
    let mut chosen = lobby
        .chosen_in_slot_order()
        .into_iter()
        .map(|target| (target.champion_id, target.package_entry_id()))
        .filter(|&(champ_id, entry_id)| {
            !is_classic(champ_id) && !dekan_core::selection::is_base_skin(entry_id, champ_id)
        });
    let first = chosen.next();
    let second = chosen.next();
    let champions = lobby.champions();
    let champ_id = first
        .map(|(champ_id, _)| champ_id)
        .or_else(|| champions.first().copied())?;
    let mods = lobby_mods_fingerprint(&state.mods, &champions);
    if first.is_none() && mods == 0 {
        return None;
    }
    Some(ArmKey {
        champ_id,
        entry_id: first.map(|(_, entry)| entry),
        mods,
        classic_slot: None,
        party: 0,
        second,
        lobby: true,
    })
}

pub(crate) fn party_skins(state: &dekan_core::state::AppState) -> dekan_core::party::VerifiedParty {
    dekan_core::party::verified_party_skins(
        &state.party_peers,
        &state.team,
        state.local_puuid.as_deref(),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WantedSkin {
    Unknown,

    Nothing,

    Skin(ArmKey),
}

pub(crate) fn wanted_skin(state: &dekan_core::state::AppState) -> WantedSkin {
    if arms_in_lobby(state) {
        return match lobby_arm_key(state) {
            Some(key) => WantedSkin::Skin(key),
            None => WantedSkin::Nothing,
        };
    }
    let Some(champ_id) = state.champion_id else {
        let nothing_at_all = state.overlay_target.is_none() && state.mods.fingerprint(None) == 0;
        return if nothing_at_all {
            WantedSkin::Nothing
        } else {
            WantedSkin::Unknown
        };
    };

    let entry_id = state
        .overlay_target
        .as_ref()
        .filter(|target| target.matches_champion(champ_id))
        .map(dekan_core::overlay::OverlayTarget::package_entry_id)
        .filter(|entry_id| !dekan_core::selection::is_base_skin(*entry_id, champ_id));

    let (party, _) = party_skins(state);
    match build_key(
        champ_id,
        entry_id,
        &state.mods,
        state.selected_skin_id,
        dekan_core::party::party_fingerprint(&party),
    ) {
        Some(key) => WantedSkin::Skin(key),
        None => WantedSkin::Nothing,
    }
}

pub(crate) fn should_disarm(wanted: WantedSkin, key: ArmKey) -> bool {
    match wanted {
        WantedSkin::Unknown => false,
        WantedSkin::Nothing => true,
        WantedSkin::Skin(wanted) => !wanted.same_own_pick(&key),
    }
}

pub(crate) enum ArmDecision {
    Settled,

    Schedule(ArmRequest),
}

pub(crate) fn arm_decision(wanted: WantedSkin, current: Option<ArmKey>) -> ArmDecision {
    let WantedSkin::Skin(key) = wanted else {
        return ArmDecision::Settled;
    };
    if current == Some(key) {
        return ArmDecision::Settled;
    }
    let debounce = if current.is_none() {
        INITIAL_ARM_DEBOUNCE
    } else {
        ARM_DEBOUNCE
    };
    ArmDecision::Schedule(ArmRequest {
        key,
        due: tokio::time::Instant::now() + debounce,
    })
}
