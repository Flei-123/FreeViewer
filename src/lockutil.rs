//! Poison-tolerant locking helpers.
//!
//! A `std::sync::Mutex` is poisoned when a thread panics while holding it.
//! `lock().unwrap()` then panics in every other thread too and takes the whole
//! process down. For the plain state guarded in this program (status strings,
//! queues, caches) the data stays usable, so these helpers take the inner
//! guard instead of propagating the panic.
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub trait LockExt<T> {
    /// Lock, ignoring poisoning.
    fn lock_ok(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_ok(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub trait RwLockExt<T> {
    /// Read lock, ignoring poisoning.
    fn read_ok(&self) -> RwLockReadGuard<'_, T>;
    /// Write lock, ignoring poisoning.
    fn write_ok(&self) -> RwLockWriteGuard<'_, T>;
}

impl<T> RwLockExt<T> for RwLock<T> {
    fn read_ok(&self) -> RwLockReadGuard<'_, T> {
        self.read().unwrap_or_else(|e| e.into_inner())
    }
    fn write_ok(&self) -> RwLockWriteGuard<'_, T> {
        self.write().unwrap_or_else(|e| e.into_inner())
    }
}
