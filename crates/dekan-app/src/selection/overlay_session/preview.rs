use super::*;

pub(super) async fn next_preview(
    fetches: &mut Option<PreviewFetches>,
) -> Option<(u32, Result<std::sync::Arc<[u8]>, String>)> {
    match fetches {
        Some(stream) => futures_util::StreamExt::next(stream).await,
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
        if self.preview_fetches.is_some() {
            debug!(
                chroma_id,
                asked = self.preview_tally.asked,
                fetched = self.preview_tally.fetched,
                failed = self.preview_tally.failed,
                "Chroma preview asked while previews are still being fetched; it is shown when it arrives"
            );
            return;
        }
        let Some(path) = catalog.and_then(|c| c.chroma_preview_path(chroma_id)) else {
            debug!(chroma_id, "Chroma preview asked for an entry without one");
            return;
        };
        debug!(chroma_id, path, "Chroma preview fetched again on hover");
        self.start_preview_fetches(vec![(chroma_id, path.to_owned())]);
    }

    pub(super) fn start_preview_fetches(&mut self, previews: Vec<(u32, String)>) {
        if previews.is_empty() {
            return;
        }
        self.preview_tally = PreviewTally {
            asked: previews.len(),
            started: Some(std::time::Instant::now()),
            ..PreviewTally::default()
        };
        self.preview_fetches = Some(catalog::chroma_preview_fetches(previews));
    }

    pub(super) fn deliver_chroma_preview(&mut self, chroma_id: u32, image: std::sync::Arc<[u8]>) {
        self.preview_tally.fetched += 1;
        self.preview_tally.bytes += image.len();
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
