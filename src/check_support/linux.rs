mod membarrier {

    /// Calls the `sys_membarrier` system call.
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
            || ret & libc::MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED as libc::c_long == 0
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
        if sys_membarrier(libc::MEMBARRIER_CMD_PRIVATE_EXPEDITED) < 0 {
            panic!(
                "Membarrier syscall failed: {}",
                std::io::Error::last_os_error()
            );
        }
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
        #[track_caller]
        fn barrier(&self) {
            unsafe {
                let page = self.page.load(Ordering::SeqCst);
                let page_size = self.page_size.load(Ordering::SeqCst);

                assert!(
                    !page.is_null() && page_size != 0,
                    "Mprotect barrier is not initialized"
                );

                // Lock the mutex.
                self.lock.clear_poison(); // Ignore poisoning
                let Ok(_guard) = self.lock.lock() else {
                    panic!("Mprotect barrier mutex is poisoned") // Should never happen
                };

                // Set the page access protections to read + write.
                if libc::mprotect(page, page_size, libc::PROT_READ | libc::PROT_WRITE) != 0 {
                    panic!(
                        "Mprotect barrier first mprotect failed: {}",
                        std::io::Error::last_os_error()
                    );
                }

                // Ensure that the page is dirty before we change the protection so that we
                // prevent the OS from skipping the global TLB flush.
                let atomic_usize = &*(page as *const AtomicUsize);
                atomic_usize.fetch_add(1, Ordering::SeqCst);

                // Set the page access protections to none.
                //
                // Changing a page protection from read + write to none causes the OS to issue
                // an interrupt to flush TLBs on all processors. This also results in flushing
                // the processor buffers.
                if libc::mprotect(page, page_size, libc::PROT_NONE) != 0 {
                    panic!(
                        "Mprotect barrier second mprotect failed: {}",
                        std::io::Error::last_os_error()
                    );
                }

                // Guard is dropped and mutex is unlocked
            }
        }

        /// An alternative solution to `sys_membarrier` that works on older Linux kernels and
        /// x86/x86-64 systems.
        fn init_barrier(&self) {
            #[cold]
            fn fatal_assert(cond: bool, msg: &'static str) {
                if !cond {
                    unsafe {
                        libc_print::libc_eprintln!("{}", msg);
                        libc::abort();
                    }
                }
            }

            #[cold]
            fn fatal_assert_print_errno(cond: bool, c_str_msg: &'static [u8]) {
                if !cond {
                    unsafe {
                        if let Some(b'\0') = c_str_msg.last() {
                            libc::perror(c_str_msg.as_ptr() as *const libc::c_char);
                        } else {
                            // Should never happen
                            libc::perror(ptr::null()); // Still print the system error
                            libc_print::libc_eprintln!(
                                "Invalid error string, missing NUL terminator (this is a bug in {} crate, please report it!)",
                                env!("CARGO_CRATE_NAME")
                            );
                        }
                        libc::abort();
                    }
                }
            }

            unsafe {
                fatal_assert(
                    self.page.load(Ordering::SeqCst).is_null(),
                    "Mprotect barrier is already initialized",
                );

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
                fatal_assert_print_errno(
                    page != libc::MAP_FAILED,
                    b"Mprotect barrier mmap failed\0",
                );
                fatal_assert(
                    (page as libc::size_t).is_multiple_of(page_size),
                    "Mprotect barrier mmap failed: returned page is not aligned",
                );

                // Locking the page ensures that it stays in memory during the two mprotect
                // calls in `Barrier::barrier()`. If the page was unmapped between those calls,
                // they would not have the expected effect of generating IPI.
                fatal_assert_print_errno(
                    libc::mlock(page, page_size) == 0,
                    b"Mprotect barrier mlock failed\0",
                );

                self.page.store(page, Ordering::SeqCst);
                self.page_size.store(page_size, Ordering::SeqCst);
            }
        }
    }

    /// Executes a heavy `mprotect`-based barrier.
    #[inline(always)]
    #[track_caller]
    pub fn barrier() {
        BARRIER.barrier();
    }

    /// Initializes the `mprotect`-based barrier.
    #[inline]
    pub fn init_barrier() {
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

pub(super) struct MembarrierImpl;

impl super::Membarrier for MembarrierImpl {
    #[inline(always)]
    fn is_supported() -> bool {
        membarrier::is_supported()
    }

    #[inline(always)]
    fn init_fallback_barrier() {
        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        mprotect::init_barrier();
    }

    #[inline(always)]
    #[track_caller]
    fn barrier() {
        membarrier::barrier();
    }

    #[inline(always)]
    #[track_caller]
    fn fallback_barrier() {
        cfg_if::cfg_if! {
            if #[cfg(any(target_arch = "x86", target_arch = "x86_64"))] {
                mprotect::barrier();
            } else {
                use std::sync::atomic;
                atomic::fence(atomic::Ordering::SeqCst);
            }
        }
    }
}
