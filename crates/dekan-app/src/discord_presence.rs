use std::time::Duration;

use dekan_core::state::{AppState, StateReceiver};
use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

const APPLICATION_ID: &str = "1560203618194292858";
const DEKAN_LOGO: &str = "3e2e030c-3aeb-4814-82d9-1a301b1322a1";

const DOWNLOAD_URL: &str = "https://github.com/chrisssst/Dekan/releases/latest";
const DISCORD_URL: &str = "https://discord.gg/kutsal";

struct PresenceInfo {
    details: String,
    state: String,
    image: String,
    image_text: String,
}

async fn presence_info(state: &AppState) -> PresenceInfo {
    let Some(champion_id) = state.champion_id else {
        let status = if state.phase.is_champ_select() {
            "Şampiyon seçiminde"
        } else if state.phase.is_in_game() {
            "Oyunda"
        } else {
            "League of Legends"
        };

        return PresenceInfo {
            details: "Dekan aktif".to_owned(),
            state: status.to_owned(),
            image: DEKAN_LOGO.to_owned(),
            image_text: "Dekan".to_owned(),
        };
    };

    let mut champion_name = format!("Şampiyon #{champion_id}");
    let mut skin_name = None;
    let mut skin_image = None;

    if let Ok(client) = dekan_lcu::client::LcuClient::discover().await {
        if let Ok(assets) = client.get_champion_assets(champion_id).await {
            if !assets.name.is_empty() {
                champion_name = assets.name.clone();
            }

            // Önce Dekan içinde seçilen kostümü kullan.
            let selected_entry = state
                .overlay_target
                .as_ref()
                .filter(|target| target.champion_id == champion_id)
                .map(|target| target.package_entry_id())
                .or(state.selected_skin_id);

            if let Some(entry_id) = selected_entry {
                skin_name = assets
                    .name_of(entry_id)
                    .filter(|name| !name.is_empty())
                    .map(str::to_owned);
            }

            // Görselde chroma olsa bile ana skin splash'ını kullan.
            let selected_skin = state
                .overlay_target
                .as_ref()
                .filter(|target| target.champion_id == champion_id)
                .map(|target| target.skin_id)
                .or(state.selected_skin_id);

            if let Some(skin_id) = selected_skin {
                if !assets.alias.is_empty() {
                    let skin_number = skin_id.saturating_sub(champion_id * 1000);

                    skin_image = Some(format!(
                        "https://ddragon.leagueoflegends.com/cdn/img/champion/splash/{}_{}.jpg",
                        assets.alias, skin_number
                    ));
                }
            }
        }
    }

    let details = skin_name.clone().unwrap_or_else(|| champion_name.clone());

    let status = if state.phase.is_in_game() {
        format!("{champion_name} ile oyunda")
    } else if state.phase.is_champ_select() {
        format!("{champion_name} seçiliyor")
    } else {
        format!("{champion_name} seçili")
    };

    PresenceInfo {
        image_text: skin_name.unwrap_or_else(|| champion_name.clone()),
        details,
        state: status,
        image: skin_image.unwrap_or_else(|| DEKAN_LOGO.to_owned()),
    }
}

pub async fn run(mut state_rx: StateReceiver, shutdown: CancellationToken) {
    loop {
        let mut client = DiscordIpcClient::new(APPLICATION_ID);

        match client.connect() {
            Ok(()) => {
                info!("Discord Rich Presence connected");

                let mut last_state = None;

                loop {
                    let state = state_rx.borrow().clone();

                    let current = (
                        state.phase,
                        state.champion_id,
                        state.selected_skin_id,
                        state.overlay_target.clone(),
                    );

                    if last_state.as_ref() != Some(&current) {
                        let info = presence_info(&state).await;

                        let presence = activity::Activity::new()
                            .activity_type(activity::ActivityType::Playing)
                            .details(info.details)
                            .state(info.state)
                            .assets(
                                activity::Assets::new()
                                    .large_image(info.image)
                                    .large_text(info.image_text)
                                    .small_image(DEKAN_LOGO)
                                    .small_text("Dekan"),
                            )
                            .buttons(vec![
                                activity::Button::new("Dekan'ı İndir", DOWNLOAD_URL),
                                activity::Button::new("Discord'a Katıl", DISCORD_URL),
                            ]);

                        if let Err(e) = client.set_activity(presence) {
                            warn!(error = %e, "Could not update Discord Rich Presence");
                            break;
                        }

                        last_state = Some(current);
                    }

                    tokio::select! {
                                            _ = shutdown.cancelled() => {
                    // ignore-ok: Discord cleanup is best-effort during shutdown
                    let _ = client.clear_activity();
                    // ignore-ok: Discord cleanup is best-effort during shutdown
                    let _ = client.close();
                                                return;
                                            }

                                            changed = state_rx.changed() => {
                                                if changed.is_err() {
                    // ignore-ok: Discord cleanup is best-effort during shutdown
                    let _ = client.clear_activity();
                    // ignore-ok: Discord cleanup is best-effort during shutdown
                    let _ = client.close();
                                                    return;
                                                }
                                            }
                                        }
                }
            }

            Err(e) => {
                warn!(
                    error = %e,
                    "Discord is unavailable; Rich Presence will retry"
                );
            }
        }

        tokio::select! {
            _ = shutdown.cancelled() => return,
            () = tokio::time::sleep(Duration::from_secs(30)) => {}
        }
    }
}
