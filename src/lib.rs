//! Process-wide memory barrier.
//!
//! Memory barrier is one of the strongest synchronization primitives in modern relaxed-memory
//! concurrency. In relaxed-memory concurrency, two threads may have different viewpoint on the
//! underlying memory system, e.g. thread T1 may have recognized a value V at location X, while T2
//! does not know of X=V at all. This discrepancy is one of the main reasons why concurrent
//! programming is hard. Memory barrier synchronizes threads in such a way that after memory
//! barriers, threads have the same viewpoint on the underlying memory system.
//!
//! Unfortunately, memory barrier is not cheap. Usually, in modern computer systems, there's a
//! designated memory barrier instruction, e.g. `MFENCE` in x86 and `DMB SY` in ARM, and they may
//! take more than 100 cycles. Use of memory barrier instruction may be tolerable for several use
//! cases, e.g. context switching of a few threads, or synchronizing events that happen only once in
//! the lifetime of a long process. However, sometimes memory barrier is necessary in a fast path,
//! which significantly degrades the performance.
//!
//! In order to reduce the synchronization cost of memory barrier, Linux and Windows provides
//! *process-wide memory barrier*, which basically performs memory barrier for every thread in the
//! process. Provided that it's even slower than the ordinary memory barrier instruction, what's the
//! benefit? At the cost of process-wide memory barrier, other threads may be exempted form issuing
//! a memory barrier instruction at all! In other words, by using process-wide memory barrier, you
//! can optimize fast path at the performance cost of slow path.
//!
//! This crate provides an abstraction of process-wide memory barrier over different operating
//! systems and hardware. It is implemented as follows. For recent Linux systems, we use the
//! `sys_membarrier()` system call; and for those old Linux systems without support for
//! `sys_membarrier()`, we fall back to the `mprotect()` system call that is known to provide
//! process-wide memory barrier semantics. For Windows, we use the `FlushProcessWriteBuffers()`
//! API. For all the other systems, we fall back to the normal `SeqCst` fence for both fast and slow
//! paths.
//!
//!
//! # Usage
//!
//! Use this crate as follows:
//!
//! ```
//! extern crate membarrier;
//! use std::sync::atomic::{fence, Ordering};
//!
//! membarrier::light();     // light-weight barrier
//! membarrier::heavy();     // heavy-weight barrier
//! fence(Ordering::SeqCst); // normal barrier
//! ```
//!
//! # Semantics
//!
//! Formally, there are three kinds of memory barrier: light one (`membarrier::light()`), heavy one
//! (`membarrier::heavy()`), and the normal one (`fence(Ordering::SeqCst)`). In an execution of a
//! program, there is a total order over all instances of memory barrier. If thread A issues barrier
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

#[allow(unused_macros)]
macro_rules! fatal_assert {
    ($cond:expr) => {
        if !$cond {
            #[allow(unused_unsafe)]
            unsafe {
                libc::abort();
            }
        }
    };
}

cfg_if::cfg_if! {
    if #[cfg(target_os = "linux")] {
        pub use crate::linux::*;
    } else if #[cfg(windows)] {
        pub use crate::windows::*;
    } else {
        pub use crate::default::*;
    }
}

#[allow(dead_code)]
mod default {
    use core::sync::atomic::{fence, Ordering};

    /// Issues a light memory barrier for fast path.
    ///
    /// It just issues the normal memory barrier instruction.
    #[inline]
    pub fn light() {
        fence(Ordering::SeqCst);
    }

    /// Issues a heavy memory barrier for slow path.
    ///
    /// It just issues the normal memory barrier instruction.
    #[inline]
    pub fn heavy() {
        fence(Ordering::SeqCst);
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use core::sync::atomic;
    use std::sync::atomic::{AtomicBool, Ordering};
    use crossbeam_utils::CachePadded;

    /// Whether the `membarrier` system call is supported.
    ///
    /// If not supported, a fallback implementation is used (`mprotect`-based trick on x86
    /// and x86_64, otherwise `SeqCst` fences).
    static MEMBARRIER_SUPPORTED: CachePadded<AtomicBool> = CachePadded::new(AtomicBool::new(false));

    #[cfg(not(test))] // Prefer manual initialization in tests
    #[ctor::ctor]
    unsafe fn check_supported_membarrier() {
        if membarrier::is_supported() {
            MEMBARRIER_SUPPORTED.store(true, Ordering::SeqCst);
        } else {
            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            mprotect::init_barrier();
        }
    }

    mod membarrier {

        /// Call the `sys_membarrier` system call.
        #[inline]
        fn sys_membarrier(cmd: libc::c_int) -> libc::c_long {
            unsafe { libc::syscall(libc::SYS_membarrier, cmd, 0 as libc::c_int) }
        }

        /// Returns `true` if the `sys_membarrier` call is available.
        pub fn is_supported() -> bool {
            // Queries which membarrier commands are supported. Checks if private expedited
            // membarrier is supported.
            let ret = sys_membarrier(libc::MEMBARRIER_CMD_QUERY);
            if ret < 0
                || ret & libc::MEMBARRIER_CMD_PRIVATE_EXPEDITED as libc::c_long == 0
                || ret & libc::MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED as libc::c_long
                    == 0
            {
                return false;
            }

            // Registers the current process as a user of private expedited membarrier.
            if sys_membarrier(libc::MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED) < 0 {
                return false;
            }

            true
        }

        /// Executes a heavy `sys_membarrier`-based barrier.
        #[inline]
        pub fn barrier() {
            fatal_assert!(sys_membarrier(libc::MEMBARRIER_CMD_PRIVATE_EXPEDITED) >= 0);
        }

        #[cfg(test)]
        mod tests {
            #[test]
            fn test_membarrier() {
                assert!(super::is_supported());
                super::barrier();
            }
        }
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    mod mprotect {
        use std::ptr;
        use std::sync::Mutex;
        use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

        struct Barrier {
            lock: Mutex<()>,
            page: AtomicPtr<libc::c_void>,
            page_size: AtomicUsize, // Use Atomic<libc::size_t> when generic_atomic is stabilized
        }

        unsafe impl Sync for Barrier {}
        unsafe impl Send for Barrier {}

        static BARRIER: Barrier = Barrier {
            lock: Mutex::new(()),
            page: AtomicPtr::new(ptr::null_mut()),
            page_size: AtomicUsize::new(0),
        };

        impl Barrier {
            /// Issues a process-wide barrier by changing access protections of a single mmap-ed
            /// page. This method is not as fast as the `sys_membarrier()` call, but works very
            /// similarly.
            #[inline(never)]
            fn barrier(&self) {
                unsafe {
                    let page = self.page.load(Ordering::SeqCst);
                    let page_size = self.page_size.load(Ordering::SeqCst);

                    fatal_assert!(!page.is_null());

                    // Lock the mutex.
                    let _guard = self.lock.lock();

                    // Set the page access protections to read + write.
                    fatal_assert!(
                        libc::mprotect(page, page_size, libc::PROT_READ | libc::PROT_WRITE,)
                            == 0
                    );

                    // Ensure that the page is dirty before we change the protection so that we
                    // prevent the OS from skipping the global TLB flush.
                    let atomic_usize = &*(page as *const AtomicUsize);
                    atomic_usize.fetch_add(1, Ordering::SeqCst);

                    // Set the page access protections to none.
                    //
                    // Changing a page protection from read + write to none causes the OS to issue
                    // an interrupt to flush TLBs on all processors. This also results in flushing
                    // the processor buffers.
                    fatal_assert!(libc::mprotect(page, page_size, libc::PROT_NONE) == 0);

                    // Guard is dropped and mutex is unlocked
                }
            }

            /// An alternative solution to `sys_membarrier` that works on older Linux kernels and
            /// x86/x86-64 systems.
            fn init_barrier(&self) {
                unsafe {
                    fatal_assert!(self.page.load(Ordering::SeqCst).is_null());

                    // Find out the page size on the current system.
                    let page_size = libc::sysconf(libc::_SC_PAGESIZE);
                    let page_size = if page_size > 0 {
                        page_size as libc::size_t
                    } else {
                        0x1000 as libc::size_t
                    };

                    // Create a dummy page.
                    let page = libc::mmap(
                        ptr::null_mut(),
                        page_size,
                        libc::PROT_READ | libc::PROT_WRITE,
                        libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                        -1 as libc::c_int,
                        0 as libc::off_t,
                    );
                    fatal_assert!(page != libc::MAP_FAILED);
                    fatal_assert!(page as libc::size_t % page_size == 0);

                    // Locking the page ensures that it stays in memory during the two mprotect
                    // calls in `Barrier::barrier()`. If the page was unmapped between those calls,
                    // they would not have the expected effect of generating IPI.
                    fatal_assert!(libc::mlock(page, page_size) == 0);

                    self.page.store(page, Ordering::SeqCst);
                    self.page_size.store(page_size, Ordering::SeqCst);
                }
            }
        }

        /// Executes a heavy `mprotect`-based barrier.
        #[inline]
        pub fn barrier() {
            BARRIER.barrier();
        }

        /// Initializes the `mprotect`-based barrier.
        #[inline]
        pub(super) fn init_barrier() {
            BARRIER.init_barrier();
        }

        #[cfg(test)]
        mod tests {
            #[test]
            fn test_mprotect() {
                super::init_barrier();
                super::barrier();
            }
        }
    }

    /// Issues a light memory barrier for fast path.
    ///
    /// It issues a compiler fence, which disallows compiler optimizations across itself. It incurs
    /// basically no costs in run-time.
    #[inline]
    pub fn light() {
        // On x86 and x86_64 mprotect is always available as fallback.
        // On other platforms, use a relaxed load to reduce the overhead to a minimum.
        #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
        if !MEMBARRIER_SUPPORTED.load(Ordering::Relaxed) {
            cold();
            atomic::fence(Ordering::SeqCst);
        }

        atomic::compiler_fence(Ordering::SeqCst)
    }

    /// Issues a heavy memory barrier for slow path.
    ///
    /// It issues a private expedited membarrier using the `sys_membarrier()` system call, if
    /// supported; otherwise, it falls back to `mprotect()`-based process-wide memory barrier.
    #[inline]
    pub fn heavy() {
        if MEMBARRIER_SUPPORTED.load(Ordering::SeqCst) {
            membarrier::barrier()
        } else {
            cold();
            cfg_if::cfg_if! {
                if #[cfg(any(target_arch = "x86", target_arch = "x86_64"))] {
                    mprotect::barrier()
                } else {
                    atomic::fence(atomic::Ordering::SeqCst)
                }
            }
        }
    }

    #[cold]
    #[inline(always)]
    fn cold() {}
}

#[cfg(windows)]
mod windows {
    use core::sync::atomic;
    use windows_sys;

    /// Issues light memory barrier for fast path.
    ///
    /// It issues compiler fence, which disallows compiler optimizations across itself.
    #[inline]
    pub fn light() {
        atomic::compiler_fence(atomic::Ordering::SeqCst);
    }

    /// Issues heavy memory barrier for slow path.
    ///
    /// It invokes the `FlushProcessWriteBuffers()` system call.
    #[inline]
    pub fn heavy() {
        unsafe {
            windows_sys::Win32::System::Threading::FlushProcessWriteBuffers();
        }
    }
}
