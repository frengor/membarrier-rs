use core::sync::atomic;
use windows_sys;

pub(super) struct BarrierImpl;

impl crate::Barrier for BarrierImpl {
    /// Issues light memory barrier for fast path.
    ///
    /// It issues compiler fence, which disallows compiler optimizations across itself.
    #[inline(always)]
    fn light() {
        atomic::compiler_fence(atomic::Ordering::SeqCst);
    }

    /// Issues heavy memory barrier for slow path.
    ///
    /// It invokes the `FlushProcessWriteBuffers()` system call.
    #[inline]
    fn heavy() {
        unsafe {
            windows_sys::Win32::System::Threading::FlushProcessWriteBuffers();
        }
    }
}
