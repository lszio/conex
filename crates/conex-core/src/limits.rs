//! Concurrency and queue admission with RAII guards and remaining-deadline waits.
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use conex_proto::v1;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

use crate::types::{CallError, CallResult};

#[derive(Debug, Clone, Copy)]
pub struct HostLimits {
    pub global_max_inflight: usize,
    pub provider_max_inflight: usize,
    pub provider_max_queue_len: usize,
    pub provider_max_queue_bytes: usize,
    pub default_timeout: Duration,
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            global_max_inflight: 32,
            provider_max_inflight: 4,
            provider_max_queue_len: 32,
            provider_max_queue_bytes: 8 * 1024 * 1024,
            default_timeout: Duration::from_secs(8),
        }
    }
}

pub struct ProviderBudget {
    semaphore: Arc<Semaphore>,
    queued: AtomicUsize,
    queued_bytes: AtomicUsize,
    max_queue_len: usize,
    max_queue_bytes: usize,
}

impl ProviderBudget {
    pub fn new(max_inflight: usize, max_queue_len: usize, max_queue_bytes: usize) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(max_inflight)),
            queued: AtomicUsize::new(0),
            queued_bytes: AtomicUsize::new(0),
            max_queue_len,
            max_queue_bytes,
        }
    }

    pub async fn acquire(&self, bytes: usize, deadline: Instant) -> CallResult<ProviderGuard> {
        let queued = self.queued.fetch_add(1, Ordering::SeqCst) + 1;
        if queued > self.max_queue_len {
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return Err(CallError::new(
                v1::ErrorCode::QuotaExceeded,
                "provider queue is full",
            ));
        }
        let queued_bytes = self.queued_bytes.fetch_add(bytes, Ordering::SeqCst) + bytes;
        if queued_bytes > self.max_queue_bytes {
            self.queued_bytes.fetch_sub(bytes, Ordering::SeqCst);
            self.queued.fetch_sub(1, Ordering::SeqCst);
            return Err(CallError::new(
                v1::ErrorCode::QuotaExceeded,
                "provider queue byte budget exceeded",
            ));
        }
        let permit =
            match tokio::time::timeout_at(deadline, self.semaphore.clone().acquire_owned()).await {
                Ok(Ok(permit)) => permit,
                Ok(Err(_)) => {
                    self.release(bytes);
                    return Err(CallError::new(
                        v1::ErrorCode::Internal,
                        "provider slot closed",
                    ));
                }
                Err(_) => {
                    self.release(bytes);
                    return Err(CallError::new(
                        v1::ErrorCode::Timeout,
                        "timed out waiting for a provider slot",
                    ));
                }
            };
        self.release(bytes);
        Ok(ProviderGuard { _permit: permit })
    }

    fn release(&self, bytes: usize) {
        self.queued.fetch_sub(1, Ordering::SeqCst);
        self.queued_bytes.fetch_sub(bytes, Ordering::SeqCst);
    }
}

pub struct ProviderGuard {
    _permit: OwnedSemaphorePermit,
}

pub struct Limiter {
    global: Arc<Semaphore>,
    limits: HostLimits,
}

impl Limiter {
    pub fn new(limits: HostLimits) -> Self {
        Self {
            global: Arc::new(Semaphore::new(limits.global_max_inflight)),
            limits,
        }
    }

    pub fn limits(&self) -> HostLimits {
        self.limits
    }

    pub fn provider_budget(&self) -> ProviderBudget {
        ProviderBudget::new(
            self.limits.provider_max_inflight,
            self.limits.provider_max_queue_len,
            self.limits.provider_max_queue_bytes,
        )
    }

    pub async fn acquire_global(&self, deadline: Instant) -> CallResult<OwnedSemaphorePermit> {
        match tokio::time::timeout_at(deadline, self.global.clone().acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) => Err(CallError::new(
                v1::ErrorCode::Internal,
                "global slot closed",
            )),
            Err(_) => Err(CallError::new(
                v1::ErrorCode::Timeout,
                "timed out waiting for a global slot",
            )),
        }
    }
}
