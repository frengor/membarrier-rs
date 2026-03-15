//! Process-wide memory barrier for Linux, Windows, OSX, FreeBSD, Android, iOS,
//! [Miri](https://github.com/rust-lang/miri) and [Loom](https://github.com/tokio-rs/loom).
//!
//! Memory barrier is one of the strongest synchronization primitives in modern relaxed-memory
//! concurrency. In relaxed-memory concurrency, two threads may have different viewpoint on the
//! underlying memory system, e.g. thread T1 may have recognized a value V at location X, while T2 does
//! not know of X=V at all. This discrepancy is one of the main reasons why concurrent programming is
//! hard. Memory barrier synchronizes threads in such a way that after memory barriers, threads have the
//! same viewpoint on the underlying memory system.
//!
//! Unfortunately, memory barrier is not cheap. Usually, in modern computer systems, there's a
//! designated memory barrier instruction, e.g. `MFENCE` in x86 and `DMB SY` in ARM, and they may
//! take more than 100 cycles. Use of memory barrier instruction may be tolerable for several use
//! cases, e.g. context switching of a few threads, or synchronizing events that happen only once in
//! the lifetime of a long process. However, sometimes a memory barrier is necessary in the fast path,
//! which significantly degrades performance.
//!
//! In order to reduce the synchronization cost of memory barrier, some OSs provide
//! *process-wide memory barrier*, which basically performs a memory barrier for every thread in the
//! process. Provided that it's even slower than the ordinary memory barrier instruction, what's the
//! benefit? At the cost of process-wide memory barrier, other threads may be exempted from issuing a
//! memory barrier instruction at all! In other words, by using process-wide memory barrier, you can
//! optimize the fast path at the performance cost of the slow path.
//!
//! This crate provides an abstraction of process-wide memory barrier over different operating
//! systems and hardware. It is implemented as follows. For Linux 4.14+ and FreeBSD 14.1+ systems,
//! we use the `membarrier()` system call. On older x86 and x86_64 Linux systems without
//! support for `membarrier()`, we fall back to the `mprotect()` system call that is known to provide
//! process-wide memory barrier semantics. For Windows, we use the `FlushProcessWriteBuffers()`
//! API. For Apple systems, we call `thread_get_register_pointer_values()` for every thread.
//! For all the other systems, we fall back to the normal `SeqCst` fence for both fast and slow
//! paths.
//!
//! # Usage
//!
//! Use this crate as follows:
//!
//! ```
//! use std::sync::atomic::{fence, Ordering};
//!
//! membarrier2::light();    // light-weight barrier
//! membarrier2::heavy();    // heavy-weight barrier
//! fence(Ordering::SeqCst); // normal barrier
//! ```
//!
//! # Semantics
//!
//! Formally, there are three kinds of memory barriers: the light one `membarrier2::light()`, the heavy
//! one `membarrier2::heavy()`, and the normal one `fence(Ordering::SeqCst)`. In an execution of a
//! program, there is a total order over all instances of memory barriers. If thread A issues barrier
//! X and thread B issues barrier Y and X is ordered before Y, then A's knowledge on the underlying
//! memory system at the time of X is transferred to B after Y, provided that:
//!
//! - Either of A's or B's barrier is heavy; or
//! - Both of A's and B's barriers are normal.
//!
//! # Reference
//!
//! For more information, see the [Linux `man` page for
//! `membarrier`](http://man7.org/linux/man-pages/man2/membarrier.2.html).

#![warn(missing_docs, missing_debug_implementations)]

cfg_if::cfg_if! {
    if #[cfg(any(miri, loom))] {
        use crate::default::BarrierImpl;
    } else if #[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))] {
        mod check_support;
        use crate::check_support::BarrierImpl;
    } else if #[cfg(windows)] {
        mod windows;
        use crate::windows::BarrierImpl;
    } else if #[cfg(target_vendor = "apple")] {
        mod apple;
        use crate::apple::BarrierImpl;
    } else {
        use crate::default::BarrierImpl;
    }
}

/// Issues a light memory barrier for the fast path.
///
/// On supported systems, it issues a *compiler* fence, which disallows compiler optimizations across itself.
/// On unsupported systems, it falls back to the normal memory barrier instruction.
#[inline(always)]
pub fn light() {
    <BarrierImpl as Barrier>::light();
}

/// Issues a heavy memory barrier for the slow path.
///
/// On supported systems, it uses an OS specific syscall or API to issue a process-wide memory barrier.
/// On unsupported systems, it falls back to the normal memory barrier instruction.
#[inline(always)]
#[track_caller]
pub fn heavy() {
    <BarrierImpl as Barrier>::heavy();
}

pub(crate) trait Barrier {
    /// Issues a light memory barrier for fast path.
    fn light();

    /// Issues a heavy memory barrier for slow path.
    #[track_caller]
    fn heavy();
}

#[allow(dead_code)]
mod default {
    #[cfg(loom)]
    use loom::sync::atomic::{Ordering, fence};

    #[cfg(not(loom))]
    use core::sync::atomic::{Ordering, fence};

    pub(super) struct BarrierImpl;

    impl crate::Barrier for BarrierImpl {
        /// Issues a light memory barrier for fast path.
        ///
        /// It just issues the normal memory barrier instruction.
        #[inline(always)]
        fn light() {
            fence(Ordering::SeqCst);
        }

        /// Issues a heavy memory barrier for slow path.
        ///
        /// It just issues the normal memory barrier instruction.
        #[inline(always)]
        fn heavy() {
            fence(Ordering::SeqCst);
        }
    }
}

#[cold]
#[allow(dead_code)]
#[inline(always)]
pub(crate) fn cold() {}
