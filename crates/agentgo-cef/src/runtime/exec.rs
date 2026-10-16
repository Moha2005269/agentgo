//! Minimal single-threaded async executor for the CEF UI thread.
//!
//! Protocol handlers that need several DevTools round trips (observe, act,
//! wait, verify) are written as ordinary `async fn`s and polled here. Nothing
//! in this module is `Send`: every future runs on the CEF UI thread, which is
//! the only thread allowed to touch browser objects.

use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

type Task = Pin<Box<dyn Future<Output = ()>>>;
type Job = Box<dyn FnOnce() + Send>;

thread_local! {
    static TASKS: RefCell<Vec<Task>> = const { RefCell::new(Vec::new()) };
    static SPAWNED: RefCell<Vec<Task>> = const { RefCell::new(Vec::new()) };
}

/// Work posted from transport threads (stdio, named pipe, scheme I/O).
static INBOX: Mutex<Vec<Job>> = Mutex::new(Vec::new());

/// Queues a closure to run on the UI thread at the next tick.
pub fn post(job: impl FnOnce() + Send + 'static) {
    if let Ok(mut inbox) = INBOX.lock() {
        inbox.push(Box::new(job));
    }
}

/// Spawns a UI-thread task. Must be called on the UI thread.
pub fn spawn(future: impl Future<Output = ()> + 'static) {
    SPAWNED.with(|spawned| spawned.borrow_mut().push(Box::pin(future)));
}

/// Runs posted jobs and polls every pending task once. Returns the number of
/// tasks still pending so the caller can pick a tick cadence.
pub fn tick() -> usize {
    let jobs = INBOX
        .lock()
        .map(|mut inbox| std::mem::take(&mut *inbox))
        .unwrap_or_default();
    for job in jobs {
        job();
    }
    // Tasks are moved out while polling so a task may spawn more tasks.
    let mut tasks = TASKS.with(|tasks| std::mem::take(&mut *tasks.borrow_mut()));
    tasks.extend(SPAWNED.with(|spawned| std::mem::take(&mut *spawned.borrow_mut())));
    let mut context = Context::from_waker(Waker::noop());
    tasks.retain_mut(|task| task.as_mut().poll(&mut context) == Poll::Pending);
    let pending = tasks.len();
    TASKS.with(|slot| {
        let mut slot = slot.borrow_mut();
        tasks.append(&mut slot);
        *slot = tasks;
    });
    pending + SPAWNED.with(|spawned| spawned.borrow().len())
}

/// Resolves after `ms` milliseconds (polled by `tick`).
pub async fn sleep(ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(ms);
    std::future::poll_fn(|_| {
        if Instant::now() >= deadline {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    })
    .await
}

/// A child process that never opens a console window on the person's
/// screen (PowerShell, cmd and other console programs).
pub fn hidden_command(program: &str) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// Runs blocking work (a child process, file I/O) on its own thread and
/// waits for it without holding the UI thread. `None` after `timeout_ms`.
pub async fn blocking<T: Send + 'static>(
    timeout_ms: u64,
    job: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let cell = std::sync::Arc::new(Mutex::new(None));
    let out = cell.clone();
    std::thread::spawn(move || {
        let value = job();
        if let Ok(mut slot) = out.lock() {
            *slot = Some(value);
        }
    });
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    std::future::poll_fn(
        |_| match cell.lock().ok().and_then(|mut slot| slot.take()) {
            Some(value) => Poll::Ready(Some(value)),
            None if Instant::now() >= deadline => Poll::Ready(None),
            None => Poll::Pending,
        },
    )
    .await
}

/// A single-assignment cell a callback fills and a task awaits.
pub struct Slot<T>(Rc<RefCell<Option<T>>>);

impl<T> Clone for Slot<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(None)))
    }
}

impl<T> Slot<T> {
    /// Stores the value; later writes are ignored.
    pub fn fill(&self, value: T) {
        let mut slot = self.0.borrow_mut();
        if slot.is_none() {
            *slot = Some(value);
        }
    }

    /// Waits for the value, or `None` once `timeout_ms` elapses.
    pub async fn wait(&self, timeout_ms: u64) -> Option<T> {
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        std::future::poll_fn(|_| {
            if let Some(value) = self.0.borrow_mut().take() {
                Poll::Ready(Some(value))
            } else if Instant::now() >= deadline {
                Poll::Ready(None)
            } else {
                Poll::Pending
            }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_run_to_completion_across_ticks() {
        let slot = Slot::<u32>::default();
        let done = Rc::new(RefCell::new(None));
        let (waiter, out) = (slot.clone(), done.clone());
        spawn(async move {
            sleep(1).await;
            *out.borrow_mut() = waiter.wait(1_000).await;
        });
        assert_eq!(tick(), 1);
        post(|| {});
        slot.fill(7);
        std::thread::sleep(Duration::from_millis(3));
        while tick() > 0 {}
        assert_eq!(*done.borrow(), Some(7));
    }

    #[test]
    fn slot_times_out() {
        let done = Rc::new(RefCell::new(Some(1)));
        let out = done.clone();
        spawn(async move {
            *out.borrow_mut() = Slot::<i32>::default().wait(1).await;
        });
        std::thread::sleep(Duration::from_millis(3));
        while tick() > 0 {}
        assert_eq!(*done.borrow(), None);
    }
}
