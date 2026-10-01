use std::collections::HashSet;
use std::time::Duration;

use dekan_core::selection::{ChampionId, SkinId};
use tracing::{debug, info, warn};

use crate::client::LcuClient;

const VERIFY_ATTEMPTS: u32 = 5;

const VERIFY_INTERVAL: Duration = Duration::from_millis(100);

#[must_use]
pub fn skin_to_register(champion_id: ChampionId, entry_id: SkinId, owned: &HashSet<u32>) -> SkinId {
    if owned.contains(&entry_id) {
        entry_id
    } else {
        champion_id * 1000
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationVia {
    AlreadySelected,

    PickAction,

    MySelection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationOutcome {
    Verified {
        via: RegistrationVia,
    },

    NotApplied {
        via: RegistrationVia,
        actual: Option<SkinId>,
    },

    Refused,

    NoSession,
}

pub async fn register_skin(client: &LcuClient, skin_id: SkinId) -> RegistrationOutcome {
    let session = match client.get_champ_select_session().await {
        Ok(session) => session,
        Err(e) => {
            warn!(
                skin_id,
                error = %e,
                "No champ select session to register the skin into; the loading screen will show whatever the client already had"
            );
            return RegistrationOutcome::NoSession;
        }
    };

    let current = session.local_player().map(|p| p.selected_skin_id);
    if current == Some(skin_id) {
        debug!(skin_id, "Champ select already holds the skin to register");
        return RegistrationOutcome::Verified {
            via: RegistrationVia::AlreadySelected,
        };
    }

    let mut via = None;
    if let Some(action) = session.local_pick_action().filter(|a| !a.completed) {
        match client.set_action_skin(action.id, skin_id).await {
            Ok(true) => via = Some(RegistrationVia::PickAction),
            Ok(false) => debug!(
                action_id = action.id,
                skin_id, "Pick action refused the skin; trying my-selection"
            ),
            Err(e) => debug!(
                action_id = action.id,
                skin_id,
                error = %e,
                "Pick action PATCH failed; trying my-selection"
            ),
        }
    }
    if via.is_none() {
        match client.set_my_selection_skin(skin_id).await {
            Ok(true) => via = Some(RegistrationVia::MySelection),
            Ok(false) => {}
            Err(e) => debug!(skin_id, error = %e, "my-selection PATCH failed"),
        }
    }

    let Some(via) = via else {
        warn!(
            skin_id,
            previous = ?current,
            "The client refused the skin on both endpoints; the loading screen will not show it"
        );
        return RegistrationOutcome::Refused;
    };

    let mut actual = None;
    for _ in 0..VERIFY_ATTEMPTS {
        tokio::time::sleep(VERIFY_INTERVAL).await;
        actual = match client.get_champ_select_session().await {
            Ok(session) => session.local_player().map(|p| p.selected_skin_id),
            Err(_) => None,
        };
        if actual == Some(skin_id) {
            info!(
                skin_id,
                via = ?via,
                previous = ?current,
                "Skin registered in champ select and verified in the session"
            );
            return RegistrationOutcome::Verified { via };
        }
    }

    warn!(
        skin_id,
        via = ?via,
        actual = ?actual,
        "The client accepted the skin but the session does not show it"
    );
    RegistrationOutcome::NotApplied { via, actual }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_owned_entries_are_registered_as_themselves() {
        let owned: HashSet<u32> = [897_008, 804_015].into_iter().collect();
        assert_eq!(skin_to_register(897, 897_008, &owned), 897_008);

        assert_eq!(skin_to_register(804, 804_015, &owned), 804_015);
    }

    #[test]
    fn test_unowned_entries_fall_back_to_the_base_skin() {
        let owned: HashSet<u32> = [897_008].into_iter().collect();

        assert_eq!(skin_to_register(897, 897_011, &owned), 897_000);
        assert_eq!(skin_to_register(33, 33_023, &HashSet::new()), 33_000);
    }
}
