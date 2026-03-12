//! Support for checking whether the `membarrier` system call is supported on the system.
//!
//! If not supported, a fallback implementation is used.

use core::sync::atomic;
use crossbeam_utils::CachePadded;
use std::sync::atomic::{AtomicBool, Ordering};

cfg_if::cfg_if! {
    if #[cfg(any(target_os = "linux", target_os = "android"))] {
        mod linux;
        use linux::MembarrierImpl;
    } else if #[cfg(target_os = "freebsd")] {
        mod freebsd;
        use freebsd::MembarrierImpl;
    } else {
        compile_error!(concat!("Unsupported platform. This is a bug in ", env!("CARGO_CRATE_NAME"), " crate, please report it!"));
    }
}

/// Whether the `membarrier` system call is supported.
///
/// If not supported, a fallback implementation is used.
static MEMBARRIER_SUPPORTED: CachePadded<AtomicBool> = CachePadded::new(AtomicBool::new(false));

#[cfg(not(test))] // Prefer manual initialization in unit tests
#[ctor::ctor]
unsafe fn check_membarrier_support() {
    if <MembarrierImpl as Membarrier>::is_supported() {
        MEMBARRIER_SUPPORTED.store(true, Ordering::SeqCst);
    } else {
        <MembarrierImpl as Membarrier>::init_fallback_barrier();
    }
}

pub(crate) trait Membarrier {
    /// Returns whether the `sys_membarrier` call is available.
    ///
    /// Must not access the Rust stdlib or panic, since it is
    /// called at load time before main (using the `ctor` crate).
    #[cfg_attr(test, allow(dead_code))] // ctor not used in unit tests
    fn is_supported() -> bool;

    /// Initializes the fallback implementation (if needed) when `sys_membarrier` is not available.
    ///
    /// Must not access the Rust stdlib or panic, since it is
    /// called at load time before main (using the `ctor` crate).
    #[inline(always)]
    #[cfg_attr(test, allow(dead_code))] // ctor not used in unit tests
    fn init_fallback_barrier() {}

    /// Executes a heavy `sys_membarrier`-based barrier.
    #[track_caller]
    fn barrier();

    /// Executes the fallback for the heavy `sys_membarrier`-based barrier.
    ///
    /// The default implementation just issues the normal memory barrier instruction.
    #[inline(always)]
    #[track_caller]
    fn fallback_barrier() {
        atomic::fence(Ordering::SeqCst);
    }
}

pub(super) struct BarrierImpl;

impl crate::Barrier for BarrierImpl {
    /// Issues a light memory barrier for fast path.
    ///
    /// It issues a compiler fence, which disallows compiler optimizations across itself. It incurs
    /// basically no costs in run-time.
    #[inline(always)]
    fn light() {
        // On linux x86 and x86_64 mprotect is always available as fallback.
        // On other platforms, use a relaxed load to reduce the overhead to a minimum.
        #[cfg(not(all(
            any(target_os = "linux", target_os = "android"),
            any(target_arch = "x86", target_arch = "x86_64")
        )))]
        if !MEMBARRIER_SUPPORTED.load(Ordering::Relaxed) {
            crate::cold();
            atomic::fence(Ordering::SeqCst);
        }

        atomic::compiler_fence(Ordering::SeqCst);
    }

    /// Issues a heavy memory barrier for slow path.
    ///
    /// It issues a private expedited membarrier using the `sys_membarrier()` system call, if
    /// supported; otherwise, it falls back to `mprotect()`-based process-wide memory barrier.
    #[inline]
    #[track_caller]
    fn heavy() {
        if MEMBARRIER_SUPPORTED.load(Ordering::SeqCst) {
            <MembarrierImpl as Membarrier>::barrier();
        } else {
            crate::cold();
            <MembarrierImpl as Membarrier>::fallback_barrier();
        }
    }
}
