use core::sync::atomic;
use libc::{mach_error_string, thread_act_t, vm_deallocate};
use mach2::error::{err_get_code, err_get_sub, err_get_system};
use mach2::kern_return::{KERN_SUCCESS, kern_return_t};
use mach2::mach_port::mach_port_deallocate;
use mach2::mach_types::thread_act_array_t;
use mach2::message::mach_msg_type_number_t;
use mach2::task::task_threads;
use mach2::thread_state::thread_get_register_pointer_values;
use mach2::traps::mach_task_self;
use std::ffi::CStr;
use std::mem::MaybeUninit;

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
    /// It invokes `thread_get_register_pointer_values()` for every thread in the current task.
    #[track_caller]
    #[inline(never)]
    fn heavy() {
        // https://github.com/dotnet/runtime/blob/c290dc12ccd792d6cb431f47efc46df8cb9a7747/src/native/minipal/memorybarrierprocesswide.c#L157
        unsafe {
            let mut threads = MaybeUninit::<thread_act_array_t>::uninit();
            let mut threads_len = MaybeUninit::<mach_msg_type_number_t>::uninit();
            let machret = task_threads(
                mach_task_self(),
                threads.as_mut_ptr(),
                threads_len.as_mut_ptr(),
            );
            if machret != KERN_SUCCESS {
                panic_with_msg(machret, "task_threads failed");
            }

            let threads = threads.assume_init();
            let threads_len = threads_len.assume_init() as usize;
            assert!(!threads.is_null(), "threads ptr is null");

            const REGISTER_SIZE: usize = 128;
            let mut register_values = [MaybeUninit::<usize>::uninit(); REGISTER_SIZE];
            let mut sp = MaybeUninit::<usize>::uninit();
            let mut first_err = None; // Reclaim memory before panicking

            {
                let threads_slice = std::slice::from_raw_parts_mut(threads, threads_len);
                for &mut thread in threads_slice {
                    // Request the threads pointer values to force the thread to emit a memory barrier
                    let mut registers = REGISTER_SIZE;
                    let machret = thread_get_register_pointer_values(
                        thread,
                        sp.as_mut_ptr(),
                        &mut registers,
                        register_values.as_mut_ptr().cast(),
                    );
                    if machret == libc::KERN_INSUFFICIENT_BUFFER_SIZE {
                        crate::cold();
                        first_err = first_err
                            .or(Some((machret, "thread_get_register_pointer_values failed")));
                    }
                    let machret = mach_port_deallocate(mach_task_self(), thread);
                    if machret != KERN_SUCCESS {
                        crate::cold();
                        first_err = first_err.or(Some((machret, "mach_port_deallocate failed")));
                    }
                }
            }

            let machret = vm_deallocate(
                mach_task_self(),
                threads as libc::vm_address_t,
                (threads_len * size_of::<thread_act_t>()) as libc::vm_size_t,
            );
            if machret != KERN_SUCCESS {
                crate::cold();
                first_err = first_err.or(Some((machret, "vm_deallocate failed")));
            }
            if let Some((machret, msg)) = first_err {
                panic_with_msg(machret, msg);
            }
        }
    }
}

#[cold]
fn panic_with_msg(machret: kern_return_t, msg: &'static str) {
    unsafe {
        let err_msg: &'static CStr = CStr::from_ptr(mach_error_string(machret));
        let err_msg = err_msg.to_string_lossy();
        let sys = err_get_system(machret);
        let sub = err_get_sub(machret);
        let code = err_get_code(machret);
        panic!("{msg}: {err_msg} (os error [{sys:#x}|{sub:#x}|{code:#x}])")
    }
}
