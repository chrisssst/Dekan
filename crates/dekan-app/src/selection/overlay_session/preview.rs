use super::*;

const MAX_CACHED_PREVIEWS: usize = 512;

pub(super) async fn next_preview(
    fetches: &mut Option<PreviewFetches>,
) -> Option<(u32, Result<std::sync::Arc<[u8]>, String>)> {
    match fetches {
        Some(stream) => futures_util::StreamExt::next(stream).await,
        None => std::future::pending().await,
    }
}

pub(super) async fn next_urgent_preview(
    urgent: &mut Option<catalog::UrgentPreview>,
) -> (u32, Result<std::sync::Arc<[u8]>, String>) {
    match urgent {
        Some(task) => {
            let result = task.await;
            *urgent = None;
            result
        }
        None => std::future::pending().await,
    }
}

#[derive(Debug, Default)]
pub(super) struct PreviewTally {
    asked: usize,
    fetched: usize,
    bytes: usize,
    failed: usize,
    first_error: Option<String>,
    started: Option<std::time::Instant>,
}

impl OverlaySession {
    pub(super) fn send_chroma_preview(&mut self, chroma_id: u32, catalog: Option<&Catalog>) {
        if let Some(image) = self.chroma_previews.get(&chroma_id) {
            debug!(
                chroma_id,
                bytes = image.len(),
                "Chroma preview served from the session cache"
            );
            self.controller.set_chroma_preview(chroma_id, image.clone());
            return;
        }
        let Some(path) = catalog.and_then(|c| c.chroma_preview_path(chroma_id)) else {
            debug!(chroma_id, "Chroma preview asked for an entry without one");
            return;
        };
        debug!(chroma_id, path, "Chroma preview urgently fetched on hover");
        self.urgent_preview = Some(catalog::fetch_single_chroma_preview(
            chroma_id,
            path.to_owned(),
        ));
    }

    pub(super) fn start_preview_fetches(&mut self, previews: Vec<(u32, String)>) {
        let mut missing = Vec::with_capacity(previews.len());
        for (id, path) in previews {
            if let Some(image) = self.chroma_previews.get(&id) {
                self.controller.set_chroma_preview(id, image.clone());
            } else {
                missing.push((id, path));
            }
        }
        if missing.is_empty() {
            return;
        }
        self.preview_tally = PreviewTally {
            asked: missing.len(),
            started: Some(std::time::Instant::now()),
            ..PreviewTally::default()
        };
        self.preview_fetches = Some(catalog::chroma_preview_fetches(missing));
    }

    pub(super) fn deliver_chroma_preview(&mut self, chroma_id: u32, image: std::sync::Arc<[u8]>) {
        let first_time = !self.chroma_previews.contains_key(&chroma_id);
        if first_time {
            self.preview_tally.fetched += 1;
            self.preview_tally.bytes += image.len();
            if self.chroma_previews.len() >= MAX_CACHED_PREVIEWS {
                if let Some(&evict) = self.chroma_previews.keys().next() {
                    self.chroma_previews.remove(&evict);
                }
            }
        }
        debug!(
            chroma_id,
            bytes = image.len(),
            "Chroma preview fetched from the client"
        );
        self.controller.set_chroma_preview(chroma_id, image.clone());
        self.chroma_previews.insert(chroma_id, image);
    }

    pub(super) fn count_failed_preview(&mut self, chroma_id: u32, reason: String) {
        debug!(chroma_id, reason, "Chroma preview not fetched");
        self.preview_tally.failed += 1;
        self.preview_tally.first_error.get_or_insert(reason);
    }

    pub(super) fn finish_preview_fetches(&mut self) {
        self.preview_fetches = None;
        let tally = std::mem::take(&mut self.preview_tally);
        let elapsed_ms = tally
            .started
            .map_or(0, |started| started.elapsed().as_millis());
        if tally.asked <= 1 {
            debug!(
                fetched = tally.fetched,
                elapsed_ms,
                error = tally.first_error.as_deref().unwrap_or("-"),
                "Chroma preview fetch on hover finished"
            );
        } else if tally.failed == 0 {
            info!(
                asked = tally.asked,
                fetched = tally.fetched,
                bytes = tally.bytes,
                elapsed_ms,
                "Chroma previews fetched from the client"
            );
        } else {
            warn!(
                asked = tally.asked,
                fetched = tally.fetched,
                failed = tally.failed,
                elapsed_ms,
                first_error = tally.first_error.as_deref().unwrap_or("-"),
                "Some chroma previews could not be fetched from the client; their hover shows no image"
            );
        }
    }
}
