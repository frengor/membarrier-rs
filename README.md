# Process-wide memory barrier

[![Build Status](https://img.shields.io/github/check-runs/frengor/membarrier2/main?style=flat&label=build)](https://github.com/frengor/membarrier2)
[![Cargo](https://img.shields.io/crates/v/membarrier2.svg?style=flat)](https://crates.io/crates/membarrier2)
[![License](https://img.shields.io/crates/l/membarrier2?style=flat)](https://github.com/frengor/membarrier2#license)
[![Documentation](https://docs.rs/membarrier2/badge.svg?style=flat)](https://docs.rs/membarrier2)

Fork of `membarrier` crate with lots of improvements and support for Linux, Windows, OSX, FreeBSD,
Android, iOS, [Miri](https://github.com/rust-lang/miri) and [Loom](https://github.com/tokio-rs/loom).

Memory barrier is one of the strongest synchronization primitives in modern relaxed-memory
concurrency. In relaxed-memory concurrency, two threads may have different viewpoint on the
underlying memory system, e.g. thread T1 may have recognized a value V at location X, while T2 does
not know of X=V at all. This discrepancy is one of the main reasons why concurrent programming is
hard. Memory barrier synchronizes threads in such a way that after memory barriers, threads have the
same viewpoint on the underlying memory system.

Unfortunately, memory barrier is not cheap. Usually, in modern computer systems, there's a
designated memory barrier instruction, e.g. `MFENCE` in x86 and `DMB SY` in ARM, and they may
take more than 100 cycles. Use of memory barrier instruction may be tolerable for several use
cases, e.g. context switching of a few threads, or synchronizing events that happen only once in
the lifetime of a long process. However, sometimes a memory barrier is necessary in the fast path,
which significantly degrades performance.

In order to reduce the synchronization cost of memory barrier, some OSs provide
*process-wide memory barrier*, which basically performs a memory barrier for every thread in the
process. Provided that it's even slower than the ordinary memory barrier instruction, what's the
benefit? At the cost of process-wide memory barrier, other threads may be exempted from issuing a
memory barrier instruction at all! In other words, by using process-wide memory barrier, you can
optimize the fast path at the performance cost of the slow path.

For process-wide memory barrier, Linux 4.14+ and FreeBSD 14.1+ provide the `membarrier()` system call.
On older x86 and x86_64 Linux systems, the `mprotect()` system call (with appropriate arguments that
provide process-wide memory barrier semantics) is used. Windows provides `FlushProcessWriteBuffers()`
API. On Apple systems, `thread_get_register_pointer_values()` is called for every thread.

## Usage

Use this crate as follows:

```rust
use std::sync::atomic::{fence, Ordering};

membarrier2::light();    // light-weight barrier
membarrier2::heavy();    // heavy-weight barrier
fence(Ordering::SeqCst); // normal barrier
```

## Semantics

Formally, there are three kinds of memory barriers: the light one (`membarrier2::light()`), the heavy
one (`membarrier2::heavy()`), and the normal one (`fence(Ordering::SeqCst)`). In an execution of a
program, there is a total order over all instances of memory barriers. If thread A issues barrier X
and thread B issues barrier Y and X is ordered before Y, then A's knowledge on the underlying memory
system at the time of X is transferred to B after Y, provided that:

- Either of A's or B's barrier is heavy; or
- Both of A's and B's barriers are normal.

## Reference

For more information, see the [Linux `man` page for
`membarrier`](http://man7.org/linux/man-pages/man2/membarrier.2.html).

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate by you,
as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
