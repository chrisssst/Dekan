use super::*;

pub(super) fn prefer_lazy_wad_checks(game_dir: &Path) {
    match dekan_platform::client_settings::disable_crash_reporting(game_dir) {
        Ok(true) => info!(
            "Turned the League client's crash reporting off so the injector checks archives as the game loads them"
        ),
        Ok(false) => debug!(
            "The League client's crash reporting is already off or the client has no settings yet"
        ),
        Err(e) => warn!(
            error = %e,
            "Could not turn the League client's crash reporting off; the injector checks every archive as the match starts"
        ),
    }
}

pub(super) async fn match_ended(state_rx: &mut StateReceiver) {
    loop {
        if state_rx.borrow_and_update().phase.is_between_matches() {
            return;
        }
        if state_rx.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}

pub(super) fn note_wanted(last: &mut Option<WantedSkin>, wanted: WantedSkin, phase: GamePhase) {
    if *last != Some(wanted) {
        debug!(phase = ?phase, wanted = ?wanted, "The skin to arm for changed");
        *last = Some(wanted);
    }
}
