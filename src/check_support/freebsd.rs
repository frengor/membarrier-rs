// libc crate does not define these constants for FreeBSD
// https://github.com/freebsd/freebsd-src/blob/041e9eb1ae094a81e55fbcaba37eb2ac194658cc/sys/sys/membarrier.h
// https://github.com/freebsd/freebsd-src/blob/041e9eb1ae094a81e55fbcaba37eb2ac194658cc/sys/sys/syscall.h#L525
#[allow(non_upper_case_globals)]
const SYS_membarrier: libc::c_int = 584;
const MEMBARRIER_CMD_QUERY: libc::c_int = 0x00000000;
const MEMBARRIER_CMD_PRIVATE_EXPEDITED: libc::c_int = 0x00000008;
const MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED: libc::c_int = 0x00000010;

/// Calls the `sys_membarrier` system call.
#[inline]
fn sys_membarrier(cmd: libc::c_int) -> libc::c_int {
    unsafe { libc::syscall(SYS_membarrier, cmd, 0 as libc::c_int) }
}

pub(super) struct MembarrierImpl;

impl super::Membarrier for MembarrierImpl {
    #[inline(always)]
    fn is_supported() -> bool {
        // Queries which membarrier commands are supported. Checks if private expedited
        // membarrier is supported.
        let ret = sys_membarrier(MEMBARRIER_CMD_QUERY);
        if ret < 0
        || ret & MEMBARRIER_CMD_PRIVATE_EXPEDITED == 0
        || ret & MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED == 0
        {
            return false;
        }

        // Registers the current process as a user of private expedited membarrier.
        if sys_membarrier(MEMBARRIER_CMD_REGISTER_PRIVATE_EXPEDITED) < 0 {
            return false;
        }

        true
    }

    #[inline(always)]
    #[track_caller]
    fn barrier() {
        if sys_membarrier(MEMBARRIER_CMD_PRIVATE_EXPEDITED) < 0 {
            panic!(
                "Membarrier syscall failed: {}",
                std::io::Error::last_os_error()
            );
        }
    }
}
