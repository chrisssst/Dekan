use super::*;

pub(super) type PendingImport = futures_util::future::BoxFuture<'static, Option<FinishedImport>>;

pub(super) struct FinishedImport {
    category: ModCategory,
    champion: Option<ChampionId>,
    alias: Option<String>,
    source: PathBuf,
    outcome: Result<PathBuf, mods_store::ImportRefusal>,
    text: &'static dekan_platform::i18n::Text,
}

pub(super) async fn next_import(pending: &mut Option<PendingImport>) -> Option<FinishedImport> {
    match pending {
        Some(task) => {
            let finished = task.await;
            *pending = None;
            finished
        }
        None => std::future::pending().await,
    }
}

pub(super) async fn pick_and_import(
    owner: isize,
    own_root: PathBuf,
    category: ModCategory,
    champion: Option<ChampionId>,
    alias: Option<String>,
    text: &'static dekan_platform::i18n::Text,
) -> Option<FinishedImport> {
    let title = text.import_title;
    let picked = tokio::task::spawn_blocking(move || {
        dekan_platform::dialog::pick_file(
            owner,
            title,
            "Mods (*.fantome, *.zip, *.modpkg)",
            "*.fantome;*.zip;*.modpkg",
        )
    })
    .await;
    let source = match picked {
        Ok(Ok(Some(path))) => path,
        Ok(Ok(None)) => {
            debug!(category = ?category, "Mod import cancelled in the file dialog");
            return None;
        }
        Ok(Err(e)) => {
            warn!(error = %e, "The file dialog could not be shown; nothing imported");
            return None;
        }
        Err(e) => {
            error!(error = %e, "The file dialog task failed");
            return None;
        }
    };
    let from = source.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        mods_store::import_archive(&own_root, category, champion, &from)
    })
    .await;
    match outcome {
        Ok(outcome) => Some(FinishedImport {
            category,
            champion,
            alias,
            source,
            outcome,
            text,
        }),
        Err(e) => {
            error!(error = %e, "The mod import task failed");
            None
        }
    }
}

impl OverlaySession {
    pub(super) fn start_import(
        &mut self,
        category: ModCategory,
        champion: Option<ChampionId>,
        alias: Option<String>,
        locale: Option<&str>,
    ) {
        if self.pending_import.is_some() {
            debug!(category = ?category, "A mod import is already open; this request is ignored");
            return;
        }
        let text = dekan_platform::i18n::Language::for_locale(locale).text();
        self.pending_import = Some(Box::pin(pick_and_import(
            self.controller.window_handle(),
            self.mods.own_root.clone(),
            category,
            champion,
            alias,
            text,
        )));
    }

    pub(super) async fn finish_import(&mut self, finished: FinishedImport) {
        let FinishedImport {
            category,
            champion,
            alias,
            source,
            outcome,
            text,
        } = finished;
        match outcome {
            Ok(destination) => {
                info!(
                    category = ?category,
                    source = %source.display(),
                    destination = %destination.display(),
                    "Custom mod imported"
                );
                if let Some(champion_id) = champion {
                    let panel = self.refresh_mods(champion_id, alias).await;
                    self.controller.set_mods(panel);
                }
            }
            Err(reason) => {
                warn!(
                    category = ?category,
                    source = %source.display(),
                    reason = %reason,
                    "Custom mod import refused"
                );
                let message = dekan_platform::i18n::fill(
                    text.import_refused,
                    "reason",
                    &reason.describe(text),
                );
                let title = text.import_title;
                drop(tokio::task::spawn_blocking(move || {
                    dekan_platform::shell::message_box(title, &message);
                }));
            }
        }
    }

    pub(super) fn open_mods_folder(&self) {
        match dekan_platform::shell::open_folder(&self.mods.own_root) {
            Ok(()) => info!(folder = %self.mods.own_root.display(), "Custom mods folder opened"),
            Err(e) => warn!(error = %e, "Could not open the custom mods folder"),
        }
    }
}
