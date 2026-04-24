//! Fixed-capacity buffer pool for `RuntimeZC` zerocopy messages.
//!
//! A [`Pool`] hands out owned [`PooledMsg`] buffers drawn from a
//! pluggable [`BufRefStore`] free list; `RuntimeZC` ships
//! those buffers between actors over channels.
//!
//! Shape:
//!
//! - Configured at construction with `msg_size` (max byte
//!   length per buffer) and `msg_count` (number of
//!   pre-allocated buffers).
//! - Free list lives behind [`BufRefStore`]; swap the impl to
//!   benchmark different concurrency strategies (Mutex,
//!   lock-free queue, ring buffer, hand-rolled atomics, …).
//! - Default store is [`MutexLifo`].
//! - Retrieval order is implementation-defined — callers
//!   treat buffers as fungible.
//!
//! Call contract:
//!
//! - [`Pool::get_msg`] returns `Ok(PooledMsg)` on success, or
//!   [`PoolError::SizeTooLarge`] / [`PoolError::NoMsgs`] on
//!   failure.
//! - [`PooledMsg`] derefs to `[u8]`; handlers use `&*msg` /
//!   `&mut *msg`.
//! - [`PooledMsg::drop`] returns the underlying `Box<[u8]>`
//!   to the store.
//!
//! Non-guarantees:
//!
//! - No zeroing on drop. A reused buffer's bytes `0..new_size`
//!   may carry residual data from the previous occupant.
//!   Ping-pong workloads overwrite or ignore them, so the
//!   leak is benign here. Applications crossing trust domains
//!   need an explicit zeroize step.
//!
//! Future extension (not yet implemented):
//!
//! - Multi-size sub-pools. Small / medium / large classes so
//!   the "many small, few large" case is cheap.
//! - A single [`Pool`] will hold several [`BufRefStore`]
//!   instances, one per class.
//! - [`BufRefStore::size`] on each still returns that
//!   sub-store's capacity; [`Pool::size`] sums across them.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Mutex};

/// Nullable buffer handle — a safe "null / non-null pointer".
///
/// - Concretely `Option<Box<[u8]>>`.
/// - Rust's niche optimization collapses this to one pointer
///   at runtime: `None` is the null bit pattern, `Some(box)`
///   is the non-null heap address of a fixed-size allocation.
/// - No discriminant tag, no space overhead.
///
/// Used wherever the code needs "a buffer, or nothing":
///
/// - Inside [`PooledMsg`] so [`Drop::drop`] can
///   [`Option::take`] the box out without leaving `self` in
///   an invalid state.
/// - As [`BufRefStore::get`]'s return value so stores can
///   signal "empty".
pub type BufRef = Option<Box<[u8]>>;

/// Failure modes for [`Pool::get_msg`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolError {
    /// The requested size exceeds the pool's configured
    /// `msg_size`. `requested` is what the caller asked for;
    /// `max` is the pool's `msg_size`.
    SizeTooLarge { requested: usize, max: usize },
    /// Every buffer the pool owns is currently in flight as a
    /// live [`PooledMsg`]. Caller must wait for a drop or build
    /// a pool with more `msg_count`.
    NoMsgs,
}

impl std::fmt::Display for PoolError {
    /// Render a human-readable one-liner for logs / panics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PoolError::SizeTooLarge { requested, max } => {
                write!(f, "requested size {requested} exceeds pool msg_size {max}")
            }
            PoolError::NoMsgs => f.write_str("pool exhausted: all buffers are in flight"),
        }
    }
}

impl std::error::Error for PoolError {}

/// Thread-safe storage backend for a [`Pool`]'s free list.
///
/// - Implementations choose the retrieval order (LIFO, FIFO,
///   ring-buffer, …).
/// - [`Pool`] treats stored buffers as fungible and does not
///   depend on any particular ordering.
/// - Swapping implementations is the primary extension point
///   for benchmarking concurrency strategies.
/// - Default impl: [`MutexLifo`].
pub trait BufRefStore: Send + Sync {
    /// Build a store owning `buffers`.
    ///
    /// - Store capacity ([`BufRefStore::size`]) = `buffers.len()`.
    /// - Initial [`BufRefStore::len`] = `buffers.len()` —
    ///   the store starts full.
    fn from_buffers(buffers: Vec<Box<[u8]>>) -> Self
    where
        Self: Sized;

    /// Retrieve one buffer from the store, or `None` if empty.
    fn get(&self) -> BufRef;

    /// Return a buffer to the store.
    ///
    /// - Precondition: total `ret` calls never exceed total
    ///   `get` calls plus the store's initial capacity.
    /// - Violating the precondition panics in well-behaved
    ///   implementations.
    fn ret(&self, buf: Box<[u8]>);

    /// Current number of buffers held by the store.
    ///
    /// - Under concurrent load the value is best-effort — it
    ///   may be stale by the time the caller reads it.
    /// - Always a valid count (never torn, never beyond
    ///   [`BufRefStore::size`]).
    fn len(&self) -> usize;

    /// Maximum capacity — the count of buffers handed to
    /// [`BufRefStore::from_buffers`]. Constant for the lifetime
    /// of the store.
    fn size(&self) -> usize;

    /// `len() == 0`. Default-impl'd; override with a faster
    /// path if an impl's `len()` is not already O(1).
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Default [`BufRefStore`] — Mutex-protected `Vec`-backed LIFO.
///
/// - Simple, correct, easy to reason about.
/// - Baseline to benchmark future lock-free alternatives
///   against.
/// - `Vec::push` / `Vec::pop` give LIFO order; cost per op is
///   lock acquire + vector push-or-pop.
/// - `Vec` capacity never changes after `from_buffers` because
///   `ret` is guaranteed to be matched by a prior `get`.
pub struct MutexLifo {
    inner: Mutex<Vec<Box<[u8]>>>,
    /// Cached at construction so `size()` is O(1) and
    /// lock-free.
    size: usize,
}

impl BufRefStore for MutexLifo {
    /// Take ownership of `buffers` behind a `Mutex`. `size` is
    /// recorded once here and never changes.
    fn from_buffers(buffers: Vec<Box<[u8]>>) -> Self {
        let size = buffers.len();
        Self {
            inner: Mutex::new(buffers),
            size,
        }
    }

    /// LIFO `pop` from the free list; `None` when empty.
    fn get(&self) -> BufRef {
        #[allow(clippy::unwrap_used)]
        // OK: Mutex only poisoned by a panic while locked; the
        //   store's lock holders only push / pop / len / capacity,
        //   none of which panic.
        self.inner.lock().unwrap().pop()
    }

    /// LIFO `push` back onto the free list.
    fn ret(&self, buf: Box<[u8]>) {
        #[allow(clippy::unwrap_used)]
        // OK: see MutexLifo::get.
        self.inner.lock().unwrap().push(buf);
    }

    /// Current count of buffers sitting in the free list.
    fn len(&self) -> usize {
        #[allow(clippy::unwrap_used)]
        // OK: see MutexLifo::get.
        self.inner.lock().unwrap().len()
    }

    /// Maximum capacity — fixed at construction.
    fn size(&self) -> usize {
        self.size
    }
}

/// Internal pool storage; held by `Arc` so actor threads share
/// one free list. Keep this private — callers see only
/// [`Pool`] and [`PooledMsg`].
struct PoolInner<S: BufRefStore> {
    msg_size: usize,
    free: S,
}

/// Cheaply-cloneable handle to a shared, fixed-capacity buffer
/// pool.
///
/// - Clones share the same free list (internally `Arc`).
/// - The `S` type parameter selects the underlying
///   [`BufRefStore`] implementation.
/// - Default `S = MutexLifo`; ordinary callers write
///   `Pool::new(64, 128)` without annotations.
/// - Benchmarks opt into a different backend with
///   `Pool::<SomeOtherStore>::new(64, 128)`.
pub struct Pool<S: BufRefStore = MutexLifo> {
    inner: Arc<PoolInner<S>>,
}

// Manual `Clone` — `#[derive(Clone)]` would add a spurious
// `S: Clone` bound, but `Arc<T>` is always clone regardless of T.
impl<S: BufRefStore> Clone for Pool<S> {
    /// Cheap — clones the `Arc`, not the buffers.
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<S: BufRefStore> Pool<S> {
    /// Build a pool with `msg_count` buffers of `msg_size`
    /// bytes each.
    ///
    /// - Pre-allocates the buffers, hands them to the store
    ///   via [`BufRefStore::from_buffers`], wraps in `Arc`.
    /// - `u32` parameters bound the pool's total footprint at
    ///   the type level (`u32::MAX * u32::MAX` bytes — beyond
    ///   any realistic usage).
    /// - `msg_count == 0` is allowed — every `get_msg`
    ///   immediately returns [`PoolError::NoMsgs`]; useful
    ///   for exhaustion tests.
    pub fn new(msg_size: u32, msg_count: u32) -> Self {
        let msg_size = msg_size as usize;
        let msg_count = msg_count as usize;
        let buffers: Vec<Box<[u8]>> = (0..msg_count)
            .map(|_| vec![0u8; msg_size].into_boxed_slice())
            .collect();
        let free = S::from_buffers(buffers);
        Self {
            inner: Arc::new(PoolInner { msg_size, free }),
        }
    }

    /// Configured maximum byte length of a single message.
    pub fn msg_size(&self) -> u32 {
        self.inner.msg_size as u32
    }

    /// Total buffer capacity across the pool's store(s).
    ///
    /// - Today: `msg_count` as passed to [`Pool::new`].
    /// - Once multi-size sub-pools land: sum across all
    ///   sub-store capacities.
    pub fn size(&self) -> usize {
        self.inner.free.size()
    }

    /// Take a buffer of logical length `size` from the pool.
    ///
    /// Errors:
    ///
    /// - [`PoolError::SizeTooLarge`] if `size > msg_size`.
    /// - [`PoolError::NoMsgs`] if every buffer is in flight.
    ///
    /// On success:
    ///
    /// - Returned [`PooledMsg`] derefs to `[u8]` of length
    ///   `size`.
    /// - Underlying allocation is always exactly `msg_size`
    ///   bytes.
    /// - For reused buffers, slots `0..size` retain whatever
    ///   bytes the previous occupant wrote (no zeroing — see
    ///   module docs).
    pub fn get_msg(&self, size: usize) -> Result<PooledMsg<S>, PoolError> {
        if size > self.inner.msg_size {
            return Err(PoolError::SizeTooLarge {
                requested: size,
                max: self.inner.msg_size,
            });
        }
        let buf = self.inner.free.get().ok_or(PoolError::NoMsgs)?;
        Ok(PooledMsg {
            buf: Some(buf),
            len: size,
            pool: self.inner.clone(),
        })
    }

    /// Current count of buffers sitting unused in the pool.
    /// Useful for tests; semantics under concurrent access are
    /// best-effort.
    #[cfg(test)]
    pub(crate) fn free_len(&self) -> usize {
        self.inner.free.len()
    }
}

/// Owned byte buffer handed out by a [`Pool`].
///
/// - Implements [`Deref`] / [`DerefMut`] over `[u8]`.
/// - Handlers use `&*msg` for `&[u8]`, `&mut *msg` for
///   `&mut [u8]`.
/// - On `Drop`, the buffer returns to its pool's store via
///   [`BufRefStore::ret`].
/// - Underlying allocation is always `Pool::msg_size` bytes
///   (fixed); `len` records the logical length the caller
///   requested and bounds what `Deref` / `DerefMut` expose.
pub struct PooledMsg<S: BufRefStore = MutexLifo> {
    /// The owning buffer. `BufRef` = `Option<Box<[u8]>>`; the
    /// `None` state is only observed transiently inside
    /// [`Drop::drop`].
    buf: BufRef,
    /// Logical message length, `≤ buf.as_ref().unwrap().len()`.
    len: usize,
    /// Back-reference to the pool so `Drop` can return the
    /// buffer. Cloning this `Arc` is the only per-`get_msg`
    /// synchronization cost beyond the store's own op.
    pool: Arc<PoolInner<S>>,
}

impl<S: BufRefStore> Deref for PooledMsg<S> {
    type Target = [u8];
    /// Borrow the buffer's first `len` bytes as an `&[u8]`.
    fn deref(&self) -> &[u8] {
        #[allow(clippy::unwrap_used)]
        // OK: `buf` is only `None` between `Drop::drop`'s `take`
        //   and the end of `drop`; `Deref` cannot run during that
        //   window.
        &self.buf.as_ref().unwrap()[..self.len]
    }
}

impl<S: BufRefStore> DerefMut for PooledMsg<S> {
    /// Mutably borrow the buffer's first `len` bytes.
    fn deref_mut(&mut self) -> &mut [u8] {
        #[allow(clippy::unwrap_used)]
        // OK: see `Deref::deref`.
        &mut self.buf.as_mut().unwrap()[..self.len]
    }
}

impl<S: BufRefStore> Drop for PooledMsg<S> {
    /// Return the underlying `Box<[u8]>` to the pool's store.
    /// `buf` is left as `None`; no further access to this
    /// value is possible once `Drop` runs.
    fn drop(&mut self) {
        if let Some(buf) = self.buf.take() {
            self.pool.free.ret(buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A freshly-built pool reports its `msg_size` and its
    /// `size` (= `msg_count`); the underlying store matches.
    #[test]
    fn new_reports_msg_size_and_size() {
        let p: Pool = Pool::new(64, 4);
        assert_eq!(p.msg_size(), 64);
        assert_eq!(p.size(), 4);
        assert_eq!(p.free_len(), 4);
    }

    /// `Pool::size` matches the underlying `BufRefStore::size`
    /// and is constant even as buffers go in flight.
    #[test]
    fn pool_size_is_constant_and_matches_store() {
        let p: Pool = Pool::new(64, 4);
        assert_eq!(p.size(), 4);
        let _m = p.get_msg(8).expect("get_msg");
        // 1 buffer in flight; size() is still 4 (capacity),
        // even though len() is 3.
        assert_eq!(p.size(), 4);
        assert_eq!(p.free_len(), 3);
    }

    /// `get_msg(size)` returns a buffer of logical length `size`.
    #[test]
    fn get_msg_returns_requested_size() {
        let p: Pool = Pool::new(64, 2);
        let m = p.get_msg(16).expect("get_msg should succeed");
        assert_eq!(m.len(), 16);
    }

    /// Boundary: `size == msg_size` is allowed.
    #[test]
    fn get_msg_at_msg_size_ok() {
        let p: Pool = Pool::new(64, 1);
        let m = p.get_msg(64).expect("get_msg should succeed");
        assert_eq!(m.len(), 64);
    }

    /// Boundary: `size == 0` is allowed and yields an empty
    /// slice. Cheap to allow; no reason to forbid.
    #[test]
    fn get_msg_zero_size_ok() {
        let p: Pool = Pool::new(64, 1);
        let m = p.get_msg(0).expect("get_msg should succeed");
        assert_eq!(m.len(), 0);
    }

    /// `size > msg_size` yields `SizeTooLarge` with the
    /// requested and max values set.
    #[test]
    fn get_msg_over_msg_size_returns_size_too_large() {
        let p: Pool = Pool::new(64, 1);
        match p.get_msg(65) {
            Err(PoolError::SizeTooLarge { requested, max }) => {
                assert_eq!(requested, 65);
                assert_eq!(max, 64);
            }
            other => panic!("expected SizeTooLarge, got {:?}", other.err()),
        }
    }

    /// Once all `msg_count` buffers are in flight, `get_msg`
    /// returns `NoMsgs`.
    #[test]
    fn get_msg_on_exhaustion_returns_no_msgs() {
        let p: Pool = Pool::new(64, 2);
        let _a = p.get_msg(8).expect("first get_msg");
        let _b = p.get_msg(8).expect("second get_msg");
        assert!(matches!(p.get_msg(8), Err(PoolError::NoMsgs)));
    }

    /// A zero-capacity pool always returns `NoMsgs`.
    #[test]
    fn zero_count_pool_always_no_msgs() {
        let p: Pool = Pool::new(64, 0);
        assert_eq!(p.size(), 0);
        assert!(matches!(p.get_msg(8), Err(PoolError::NoMsgs)));
    }

    /// After dropping a live `PooledMsg`, the pool recovers a
    /// slot and `get_msg` succeeds again.
    #[test]
    fn drop_restores_slot_and_get_succeeds() {
        let p: Pool = Pool::new(64, 1);
        let m = p.get_msg(8).expect("first get_msg");
        assert!(matches!(p.get_msg(8), Err(PoolError::NoMsgs)));
        drop(m);
        let _m2 = p.get_msg(8).expect("post-drop get_msg");
    }

    /// The same `Box<[u8]>` allocation is reused across a
    /// get / drop / get cycle (LIFO, no reallocation).
    #[test]
    fn buffer_is_reused_across_get_drop_get() {
        let p: Pool = Pool::new(64, 1);
        let m1 = p.get_msg(8).expect("first get_msg");
        let ptr1 = m1.as_ptr();
        drop(m1);
        let m2 = p.get_msg(16).expect("second get_msg");
        assert_eq!(m2.as_ptr(), ptr1, "expected reuse of the same allocation");
        assert_eq!(m2.len(), 16);
    }

    /// Cloned `Pool` handles share the same free list.
    #[test]
    fn clone_pool_shares_free_list() {
        let p1: Pool = Pool::new(64, 2);
        let p2 = p1.clone();
        let m = p1.get_msg(8).expect("get_msg");
        assert_eq!(p2.free_len(), 1);
        drop(m);
        assert_eq!(p2.free_len(), 2);
    }

    /// Writes through `DerefMut` round-trip via `Deref`. The
    /// logical length caps what's visible.
    #[test]
    fn writes_round_trip() {
        let p: Pool = Pool::new(64, 1);
        let mut m = p.get_msg(4).expect("get_msg");
        m.copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(&*m, &[1, 2, 3, 4]);
    }

    /// Sharing a pool across threads is sound; buffers dropped
    /// on a worker thread return to the same free list visible
    /// from the main thread.
    #[test]
    fn pool_works_across_threads() {
        use std::thread;
        let p: Pool = Pool::new(64, 1);
        let p2 = p.clone();
        let h = thread::spawn(move || {
            let m = p2.get_msg(8).expect("worker get_msg");
            drop(m);
        });
        h.join().expect("worker thread panicked");
        assert_eq!(p.free_len(), 1);
    }

    /// `PoolError` renders a readable `Display` string for
    /// both variants.
    #[test]
    fn pool_error_display() {
        let e1 = PoolError::SizeTooLarge {
            requested: 128,
            max: 64,
        };
        assert_eq!(
            e1.to_string(),
            "requested size 128 exceeds pool msg_size 64"
        );
        let e2 = PoolError::NoMsgs;
        assert_eq!(e2.to_string(), "pool exhausted: all buffers are in flight");
    }

    /// Direct tests for `MutexLifo` as a `BufRefStore`: the
    /// trait contract (`get` / `ret` / `len` / `size` /
    /// `from_buffers`) works without a `Pool` wrapped around it.
    #[test]
    fn mutex_lifo_trait_contract() {
        let buffers: Vec<Box<[u8]>> = (0..3).map(|_| vec![0u8; 16].into_boxed_slice()).collect();
        let s = MutexLifo::from_buffers(buffers);
        assert_eq!(s.size(), 3);
        assert_eq!(s.len(), 3);

        // Drain.
        let b1 = s.get().expect("first get");
        let b2 = s.get().expect("second get");
        let b3 = s.get().expect("third get");
        assert_eq!(s.len(), 0);
        assert_eq!(s.size(), 3); // size is constant
        assert!(s.get().is_none());

        // Refill.
        s.ret(b1);
        s.ret(b2);
        s.ret(b3);
        assert_eq!(s.len(), 3);
        assert_eq!(s.size(), 3);
    }
}
