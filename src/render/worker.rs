use super::{Cancellation, Document, RenderSpec, Renderer};
use crate::{config::Config, model::Snapshot};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

/// Owned inputs sent to the background render worker.
pub struct RenderRequest {
    /// Request identifier used to cancel superseded work.
    pub generation: u64,
    /// Repository changes to render from.
    pub snapshot: Arc<Snapshot>,
    /// Indices of changes to display, in render order.
    pub selected: Vec<usize>,
    /// Available terminal width in columns.
    pub width: usize,
    /// Whether to lay out old and new lines side by side.
    pub split: bool,
    /// Whether to display source line number columns.
    pub show_line_numbers: bool,
    /// Search query to highlight in the rendered text.
    pub query: String,
    /// Initial preview row limit; zero requests a full preview.
    pub preview_rows: usize,
}
/// A rendered update tagged with its originating request.
pub struct RenderResult {
    /// Request identifier used by the UI to reject stale results.
    pub generation: u64,
    /// Latest rendered document available for display.
    pub document: Arc<Document>,
    /// Whether this is the final result for the request.
    pub finished: bool,
}

/// Request replacement is signalled before enqueueing so an in-flight syntax
/// pass can stop at the next source line, without waiting for the job to finish.
pub struct RenderWorker {
    requests: mpsc::Sender<RenderRequest>,
    results: mpsc::Receiver<RenderResult>,
    latest: Arc<AtomicU64>,
}
impl RenderWorker {
    pub fn spawn(config: Config) -> crate::AppResult<Self> {
        let (requests, incoming) = mpsc::channel::<RenderRequest>();
        let (outgoing, results) = mpsc::channel();
        let (ready, initialized) = mpsc::sync_channel(1);
        let latest = Arc::new(AtomicU64::new(0));
        let worker_latest = Arc::clone(&latest);
        thread::Builder::new()
            .name("diff-render".into())
            .spawn(move || {
                // Syntect parser states stay on their owning worker thread.
                let mut renderer = match Renderer::new(config) {
                    Ok(renderer) => {
                        if ready.send(Ok(())).is_err() {
                            return;
                        }
                        renderer
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                while let Ok(mut request) = incoming.recv() {
                    while let Ok(newer) = incoming.try_recv() {
                        request = newer;
                    }
                    let cancel = Cancellation::new(Arc::clone(&worker_latest), request.generation);
                    let spec = RenderSpec {
                        snapshot: &request.snapshot,
                        selected: &request.selected,
                        width: request.width,
                        split: request.split,
                        show_line_numbers: request.show_line_numbers,
                        query: &request.query,
                        preview_rows: request.preview_rows,
                    };
                    let _ = renderer.render_progressive(spec, &cancel, |update| {
                        cancel.check().is_ok()
                            && outgoing
                                .send(RenderResult {
                                    generation: request.generation,
                                    document: update.document,
                                    finished: update.finished,
                                })
                                .is_ok()
                    });
                }
            })?;
        initialized.recv().map_err(io::Error::other)??;
        Ok(Self {
            requests,
            results,
            latest,
        })
    }
    pub fn request(&self, request: RenderRequest) -> Result<(), mpsc::SendError<RenderRequest>> {
        self.latest.store(request.generation, Ordering::Relaxed);
        self.requests.send(request)
    }
    pub fn try_recv(&self) -> Result<RenderResult, mpsc::TryRecvError> {
        self.results.try_recv()
    }
}
impl Drop for RenderWorker {
    fn drop(&mut self) {
        self.latest.fetch_add(1, Ordering::Relaxed);
    }
}
