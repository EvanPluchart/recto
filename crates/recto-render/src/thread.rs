//! The render thread: owns the `Renderer` and serves requests over `flume` channels.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow};
use flume::{Receiver, Sender};

use crate::{Bitmap, RenderRequest, Renderer};

enum Command {
    Open { bytes: Arc<[u8]>, password: Option<String> },
    Render(RenderRequest),
}

#[derive(Debug)]
pub enum Response {
    /// Answer to `open`: the page count.
    Opened(Result<usize>),
    Rendered { request: RenderRequest, result: Result<Bitmap> },
}

pub struct RenderThread {
    commands: Sender<Command>,
    responses: Receiver<Response>,
    generation: Arc<AtomicU64>,
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
        let handle = std::thread::Builder::new()
            .name("recto-render".into())
            .spawn(move || match make() {
                Ok(renderer) => {
                    let _ = ready_tx.send(Ok(()));
                    serve(renderer, command_rx, response_tx, latest);
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(anyhow::Error::new(error)));
                }
            })
            .context("starting the render thread")?;
        ready_rx.recv().map_err(|_| anyhow!("the render thread stopped while starting"))??;
        Ok(Self { commands, responses, generation, handle: Some(handle) })
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

    /// Answers, in request order. The receiver can be cloned and awaited with `recv_async`.
    pub fn responses(&self) -> &Receiver<Response> {
        &self.responses
    }
}

impl Drop for RenderThread {
    fn drop(&mut self) {
        // Closing the command channel ends the loop; then wait for the backend to be released.
        let (closed, _) = flume::unbounded();
        drop(std::mem::replace(&mut self.commands, closed));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn serve<R: Renderer>(mut renderer: R, commands: Receiver<Command>, responses: Sender<Response>, latest: Arc<AtomicU64>) {
    for command in commands.iter() {
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
        };
        if responses.send(response).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recto_core::PageId;

    #[derive(Debug, thiserror::Error)]
    #[error("mock")]
    struct MockError;

    /// Blocks in its first `open` until released, so the test can queue requests deterministically.
    struct Mock {
        gate: Option<Receiver<()>>,
    }

    impl Renderer for Mock {
        type Error = MockError;

        fn open(&mut self, _: Arc<[u8]>, _: Option<&str>) -> Result<usize, MockError> {
            if let Some(gate) = self.gate.take() {
                let _ = gate.recv();
            }
            Ok(3)
        }

        fn render(&mut self, request: &RenderRequest) -> Result<Bitmap, MockError> {
            let width = request.index as u32 + 1;
            Ok(Bitmap { width, height: 1, pixels: vec![255; 4 * width as usize] })
        }
    }

    fn request(index: usize, generation: u64) -> RenderRequest {
        RenderRequest { page: PageId { source: 0, index: index as u32 }, index, scale: 1.0, generation }
    }

    #[test]
    fn stale_generations_are_dropped() {
        let (release, gate) = flume::bounded(1);
        let thread = RenderThread::spawn(move || Ok::<_, MockError>(Mock { gate: Some(gate) })).unwrap();
        thread.open(Arc::from(&b""[..]), None);

        // The thread is blocked in `open`: everything below is queued before it looks at any request.
        let old = thread.next_generation();
        thread.request(request(0, old));
        thread.request(request(1, old));
        let new = thread.next_generation();
        thread.request(request(2, new));
        thread.open(Arc::from(&b""[..]), None); // marks the end of the queue
        release.send(()).unwrap();

        let mut rendered = Vec::new();
        let mut opened = 0;
        while opened < 2 {
            match thread.responses().recv().unwrap() {
                Response::Opened(count) => {
                    assert_eq!(count.unwrap(), 3);
                    opened += 1;
                }
                Response::Rendered { request, result } => {
                    result.unwrap();
                    rendered.push(request);
                }
            }
        }
        assert_eq!(rendered, vec![request(2, new)]);
    }

    #[test]
    fn current_generation_is_rendered() {
        let thread = RenderThread::spawn(|| Ok::<_, MockError>(Mock { gate: None })).unwrap();
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
    fn spawn_reports_backend_errors() {
        let error = RenderThread::spawn(|| Err::<Mock, _>(MockError)).err().unwrap();
        assert_eq!(error.to_string(), "mock");
    }
}
