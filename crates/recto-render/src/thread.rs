//! The render thread: owns the `Renderer` and serves requests over `flume` channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow};
use flume::{Receiver, Sender};

use crate::{Bitmap, DocInfo, PageText, RenderRequest, Renderer};

enum Command {
    Open { bytes: Arc<[u8]>, password: Option<String> },
    Render(RenderRequest),
    Text(usize),
    Annotations(bool),
}

#[derive(Debug)]
pub enum Response {
    /// Answer to `open`.
    Opened(Result<DocInfo>),
    Rendered { request: RenderRequest, result: Result<Bitmap> },
    /// Answer to `text`. Text requests are never dropped as stale.
    Text { index: usize, result: Result<PageText> },
}

pub struct RenderThread {
    commands: Sender<Command>,
    responses: Receiver<Response>,
    generation: Arc<AtomicU64>,
    /// Set on shutdown: the thread stops at the next command instead of working through the queue.
    stopped: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl RenderThread {
    /// Starts the thread and builds the renderer on it, so the backend never crosses threads.
    /// Returns once the renderer exists, or with the error that prevented it.
    pub fn spawn<R, F>(make: F) -> Result<Self>
    where
        R: Renderer,
        F: FnOnce() -> Result<R, R::Error> + Send + 'static,
    {
        let (commands, command_rx) = flume::unbounded();
        let (response_tx, responses) = flume::unbounded();
        let (ready_tx, ready_rx) = flume::bounded(1);
        let generation = Arc::new(AtomicU64::new(0));
        let latest = generation.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let handle = std::thread::Builder::new()
            .name("recto-render".into())
            .spawn(move || match make() {
                Ok(renderer) => {
                    let _ = ready_tx.send(Ok(()));
                    serve(renderer, command_rx, response_tx, latest, stop);
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(anyhow::Error::new(error)));
                }
            })
            .context("starting the render thread")?;
        ready_rx.recv().map_err(|_| anyhow!("the render thread stopped while starting"))??;
        Ok(Self { commands, responses, generation, stopped, handle: Some(handle) })
    }

    pub fn open(&self, bytes: Arc<[u8]>, password: Option<&str>) {
        let _ = self.commands.send(Command::Open { bytes, password: password.map(str::to_owned) });
    }

    /// Starts a new generation: requests queued with an older one are dropped unrendered.
    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn request(&self, request: RenderRequest) {
        let _ = self.commands.send(Command::Render(request));
    }

    /// Asks for the text of page `index`.
    /// Draws the next pages with or without their annotations.
    pub fn show_annotations(&self, show: bool) {
        let _ = self.commands.send(Command::Annotations(show));
    }

    pub fn text(&self, index: usize) {
        let _ = self.commands.send(Command::Text(index));
    }

    /// Abandons every queued command; the backend is released once the current one ends.
    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    /// Answers, in request order. The receiver can be cloned and awaited with `recv_async`.
    pub fn responses(&self) -> &Receiver<Response> {
        &self.responses
    }
}

impl Drop for RenderThread {
    fn drop(&mut self) {
        // Stop at the next command (a closed channel alone would still deliver the queue), then
        // wait for the backend to be released.
        self.shutdown();
        let (closed, _) = flume::unbounded();
        drop(std::mem::replace(&mut self.commands, closed));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve<R: Renderer>(
    mut renderer: R,
    commands: Receiver<Command>,
    responses: Sender<Response>,
    latest: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
) {
    for command in commands.iter() {
        if stopped.load(Ordering::Acquire) {
            break;
        }
        let response = match command {
            Command::Open { bytes, password } => {
                Response::Opened(renderer.open(bytes, password.as_deref()).map_err(anyhow::Error::new))
            }
            Command::Render(request) => {
                if request.generation < latest.load(Ordering::Acquire) {
                    continue;
                }
                Response::Rendered { request, result: renderer.render(&request).map_err(anyhow::Error::new) }
            }
            Command::Text(index) => Response::Text { index, result: renderer.text(index).map_err(anyhow::Error::new) },
            Command::Annotations(show) => {
                renderer.show_annotations(show);
                continue;
            }
        };
        if responses.send(response).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Purpose;
    use recto_core::PageId;

    #[derive(Debug, thiserror::Error)]
    #[error("mock")]
    struct MockError;

    /// Blocks in its first `open` until released, so the test can queue requests deterministically.
    struct Mock {
        gate: Option<Receiver<()>>,
        /// Pages rendered so far.
        renders: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Renderer for Mock {
        type Error = MockError;

        fn open(&mut self, _: Arc<[u8]>, _: Option<&str>) -> Result<DocInfo, MockError> {
            if let Some(gate) = self.gate.take() {
                let _ = gate.recv();
            }
            Ok(DocInfo { page_sizes: vec![crate::Size { width: 10.0, height: 20.0 }; 3], outline: Vec::new() })
        }

        fn render(&mut self, request: &RenderRequest) -> Result<Bitmap, MockError> {
            self.renders.fetch_add(1, Ordering::Relaxed);
            let width = request.index as u32 + 1;
            Ok(Bitmap { width, height: 1, pixels: vec![255; 4 * width as usize] })
        }

        fn text(&mut self, _: usize) -> Result<PageText, MockError> {
            Ok(PageText::default())
        }
    }

    fn request(index: usize, generation: u64) -> RenderRequest {
        let page = PageId { source: 0, index: index as u32 };
        RenderRequest { page, index, scale: 1.0, generation, purpose: Purpose::Page, tile: None }
    }

    #[test]
    fn stale_generations_are_dropped() {
        let (release, gate) = flume::bounded(1);
        let thread = RenderThread::spawn(move || Ok::<_, MockError>(Mock { gate: Some(gate), renders: Arc::default() })).unwrap();
        thread.open(Arc::from(&b""[..]), None);

        // The thread is blocked in `open`: everything below is queued before it looks at any request.
        let old = thread.next_generation();
        thread.request(request(0, old));
        thread.request(request(1, old));
        thread.text(1);
        let new = thread.next_generation();
        thread.request(request(2, new));
        thread.open(Arc::from(&b""[..]), None); // marks the end of the queue
        release.send(()).unwrap();

        let mut rendered = Vec::new();
        let mut texts = Vec::new();
        let mut opened = 0;
        while opened < 2 {
            match thread.responses().recv().unwrap() {
                Response::Opened(count) => {
                    assert_eq!(count.unwrap().page_count(), 3);
                    opened += 1;
                }
                Response::Rendered { request, result } => {
                    result.unwrap();
                    rendered.push(request);
                }
                Response::Text { index, result } => {
                    result.unwrap();
                    texts.push(index);
                }
            }
        }
        assert_eq!(rendered, vec![request(2, new)]);
        // Text requests have no generation: queued before the new one, still answered.
        assert_eq!(texts, vec![1]);
    }

    #[test]
    fn current_generation_is_rendered() {
        let thread = RenderThread::spawn(|| Ok::<_, MockError>(Mock { gate: None, renders: Arc::default() })).unwrap();
        let generation = thread.next_generation();
        thread.request(request(1, generation));
        match thread.responses().recv().unwrap() {
            Response::Rendered { request: r, result } => {
                assert_eq!(r, request(1, generation));
                assert_eq!(result.unwrap().width, 2);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn shutdown_abandons_the_queue() {
        let (release, gate) = flume::bounded(1);
        let renders = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = renders.clone();
        let thread = RenderThread::spawn(move || Ok::<_, MockError>(Mock { gate: Some(gate), renders: counter })).unwrap();
        thread.open(Arc::from(&b""[..]), None);
        let generation = thread.next_generation();
        for index in 0..50 {
            thread.request(request(index, generation));
        }
        thread.text(0);
        thread.shutdown();
        release.send(()).unwrap();
        drop(thread); // joins: returns once `open` ends, without rendering the queue
        assert_eq!(renders.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn spawn_reports_backend_errors() {
        let error = RenderThread::spawn(|| Err::<Mock, _>(MockError)).err().unwrap();
        assert_eq!(error.to_string(), "mock");
    }
}
