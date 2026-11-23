use super::*;

impl OverlaySession {
    fn party_view(&self) -> dekan_core::overlay::InClientPartyView {
        let state = self.state_rx.borrow();
        let connected = match state.party_status {
            dekan_core::party::PartyStatus::Connected { members } => members,
            _ => 0,
        };
        let joinable_friends = if !state.party_share
            && connected >= 2
            && dekan_core::party::may_ask_to_share(state.phase)
        {
            connected - 1
        } else {
            0
        };
        let peer_identities = state
            .party_peers
            .iter()
            .filter_map(|p| {
                p.identity
                    .as_ref()
                    .filter(|id| !id.is_empty())
                    .map(|id| (p.puuid.clone(), id.clone()))
            })
            .collect();
        dekan_core::overlay::InClientPartyView {
            joinable_friends,
            connected_members: if state.party_share { connected } else { 0 },
            local_identity: state.local_identity.clone(),
            peer_identities,
        }
    }

    pub(super) async fn in_client_catalog_snapshot(
        &mut self,
        wanted: bool,
        use_in_client: bool,
        phase: GamePhase,
        catalog: Option<&Catalog>,
    ) -> Option<Catalog> {
        if !use_in_client || phase == GamePhase::InProgress {
            return None;
        }
        let party = self.party_view();
        if let Some(active) = catalog.filter(|_| wanted) {
            let mut copy = active.clone();
            copy.mods.selection = self.state_rx.borrow().mods.view(Some(copy.champion_id));
            copy.party = party;
            return Some(copy);
        }
        if !self.global_mods_loaded {
            let roots = self.mods.roots.clone();
            let scanned = tokio::task::spawn_blocking(move || {
                dekan_core::mods::scan_catalog(&roots, None, &|_| true)
            })
            .await
            .unwrap_or_default();
            self.mod_catalog = scanned;
            self.global_mods_loaded = true;
        }
        let selection = self.state_rx.borrow().mods.view(None);
        Some(Catalog {
            mods: ModsPanel {
                available: self.mod_catalog.clone(),
                selection,
            },
            party,
            ..Catalog::default()
        })
    }
}
