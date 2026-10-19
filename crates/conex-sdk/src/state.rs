//! Atomic domain capture is supplied by `State::with_handler`: attach `StateObserver`
//! and read the snapshot while holding one domain lock, then return a `StateLease`.
use crate::{
    Error, ModuleRef, Result,
    capability::validate,
    client::{Budget, Core, Queued},
};
use conex_proto::{ErrorCode, StateSample};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{mpsc, watch};

#[derive(Clone, Debug)]
pub struct Sequence {
    revision: u64,
    seq: u64,
}
impl Sequence {
    pub fn new(revision: u64) -> Self {
        Self { revision, seq: 0 }
    }
    pub fn accept(&mut self, seq: u64, revision: u64) -> Result<()> {
        if self.seq.checked_add(1) != Some(seq) || revision <= self.revision {
            return Err(Error::ended(
                ErrorCode::SessionLost,
                "non-contiguous seq or non-increasing revision",
            ));
        }
        self.seq = seq;
        self.revision = revision;
        Ok(())
    }
}

/// RAII registration owned by the SDK task, not by the provider's caller.
/// Cleanup MUST remove the observer from the domain's observer table.
pub struct StateLease {
    pub snapshot: StateSample,
    cleanup: Option<Box<dyn FnOnce() + Send>>,
}
impl StateLease {
    pub fn new(snapshot: StateSample, cleanup: impl FnOnce() + Send + 'static) -> Self {
        Self {
            snapshot,
            cleanup: Some(Box::new(cleanup)),
        }
    }
}
impl Drop for StateLease {
    fn drop(&mut self) {
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

#[derive(Clone)]
pub struct StateObserver {
    pub(crate) tx: mpsc::Sender<Queued<StateSample>>,
    pub(crate) terminal: watch::Sender<Option<Error>>,
    pub(crate) stopped: Arc<AtomicBool>,
    pub(crate) budget: Arc<Budget>,
    pub(crate) schema: String,
}
impl StateObserver {
    /// Call under the same domain lock used to allocate the revision. Never waits.
    /// Overflow terminates the observer, not the committed domain mutation.
    pub fn publish(&self, sample: StateSample) -> Result<()> {
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::unavailable());
        }
        let result = (|| {
            let json =
                conex_proto::capability::state_sample_json(&sample).map_err(Error::protocol)?;
            validate(
                &self.schema,
                json.get("value").unwrap_or(&serde_json::Value::Null),
            )?;
            let bytes = serde_json::to_vec(&json).map_err(Error::decode)?.len();
            let queued = self.budget.reserve(sample, bytes, false)?;
            self.tx
                .try_send(queued)
                .map_err(|_| Error::ended(ErrorCode::SlowConsumer, "provider observer queue full"))
        })();
        if let Err(error) = &result {
            self.end(error.clone());
        }
        result
    }
    pub fn end(&self, error: Error) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            self.terminal.send_replace(Some(error));
        }
    }
    pub fn is_closed(&self) -> bool {
        self.stopped.load(Ordering::Acquire) || self.tx.is_closed()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StateEvent {
    Snapshot(StateSample),
    Update { seq: u64, sample: StateSample },
    End(Error),
}
pub(crate) struct ConsumerState {
    pub target: ModuleRef,
    pub schema: String,
    pub events: mpsc::Sender<Queued<StateEvent>>,
    pub terminal: watch::Sender<Option<Error>>,
    pub stopped: Arc<AtomicBool>,
    pub sequence: Option<Sequence>,
}
impl ConsumerState {
    pub fn end(&self, error: Error) {
        if !self.stopped.swap(true, Ordering::AcqRel) {
            self.terminal.send_replace(Some(error));
        }
    }
}

/// Pull-based delivery: no callbacks run on the I/O owner. End is delivered once.
/// Dropping the handle immediately fences delivery and requests upstream cleanup.
pub struct Subscription {
    pub id: String,
    pub(crate) core: Weak<Core>,
    pub(crate) target: ModuleRef,
    pub(crate) events: mpsc::Receiver<Queued<StateEvent>>,
    pub(crate) terminal: watch::Receiver<Option<Error>>,
    pub(crate) stopped: Arc<AtomicBool>,
    pub(crate) ended: bool,
}
impl Subscription {
    pub async fn next(&mut self) -> Option<StateEvent> {
        if self.ended {
            return None;
        }
        loop {
            if let Some(error) = self.terminal.borrow().clone() {
                self.ended = true;
                self.events.close();
                while self.events.try_recv().is_ok() {}
                return Some(StateEvent::End(error));
            }
            tokio::select! {
                biased;
                changed = self.terminal.changed() => {
                    if changed.is_err() {
                        self.ended = true;
                        return Some(StateEvent::End(Error::ended(ErrorCode::Unavailable, "link closed")));
                    }
                }
                item = self.events.recv() => {
                    if let Some(item) = item {
                        if self.stopped.load(Ordering::Acquire) { continue; }
                        return Some(item.value);
                    }
                    self.ended = true;
                    return Some(StateEvent::End(Error::ended(ErrorCode::Unavailable, "subscription closed")));
                }
            }
        }
    }
    /// Local delivery stops synchronously, before polling the returned network cleanup.
    pub fn unsubscribe(
        &mut self,
    ) -> impl Future<Output = Result<conex_proto::StateUnsubscribeResult>> + Send + 'static {
        let already_stopped = self.stopped.load(Ordering::Acquire);
        let call = if already_stopped {
            Ok(None)
        } else {
            self.core
                .upgrade()
                .ok_or_else(Error::unavailable)
                .and_then(|core| {
                    core.end_consumer(&self.id, Error::ended(ErrorCode::Cancelled, "unsubscribed"));
                    let call = core.begin_unsubscribe(self.target.clone(), self.id.clone());
                    if call.is_err() {
                        core.shutdown();
                    }
                    call.map(Some)
                })
        };
        async move {
            match call? {
                Some(call) => call.result().await,
                None => Ok(conex_proto::StateUnsubscribeResult { removed: false }),
            }
        }
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        if let Some(core) = self.core.upgrade() {
            core.end_consumer(
                &self.id,
                Error::ended(ErrorCode::Cancelled, "subscription dropped"),
            );
            if !core.is_closed() {
                core.unsubscribe_detached(self.target.clone(), self.id.clone());
            }
        }
    }
}
