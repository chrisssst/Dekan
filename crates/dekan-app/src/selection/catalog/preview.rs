use super::*;

pub(super) const PREVIEW_FETCHES: usize = 16;

pub type PreviewFetches =
    futures_util::stream::BoxStream<'static, (u32, Result<Arc<[u8]>, String>)>;

pub type UrgentPreview = futures_util::future::BoxFuture<'static, (u32, Result<Arc<[u8]>, String>)>;

pub fn chroma_preview_fetches(previews: Vec<(u32, String)>) -> PreviewFetches {
    use futures_util::StreamExt;
    futures_util::stream::once(lcu_client())
        .flat_map(move |client| {
            futures_util::stream::iter(previews.clone())
                .map(move |(id, path)| {
                    let client = client.clone();
                    async move {
                        let image = match client {
                            Some(client) => fetch_chroma_preview(&client, &path).await,
                            None => Err("the League client could not be reached".to_owned()),
                        };
                        (id, image)
                    }
                })
                .buffer_unordered(PREVIEW_FETCHES)
        })
        .boxed()
}

pub fn fetch_single_chroma_preview(chroma_id: u32, path: String) -> UrgentPreview {
    use futures_util::FutureExt;
    async move {
        let image = match lcu_client().await {
            Some(client) => fetch_chroma_preview(&client, &path).await,
            None => Err("the League client could not be reached".to_owned()),
        };
        (chroma_id, image)
    }
    .boxed()
}

pub(super) async fn fetch_chroma_preview(
    client: &dekan_lcu::client::LcuClient,
    path: &str,
) -> Result<Arc<[u8]>, String> {
    match client.get_asset_bytes(path).await {
        Ok(bytes) if !bytes.is_empty() => Ok(Arc::from(bytes)),
        Ok(_) => Err(format!("{path} answered with no bytes")),
        Err(e) => Err(format!("{path}: {e}")),
    }
}
