use core::sync::atomic::{fence, Ordering};

#[test]
fn fences() {
    membarrier2::light();     // light-weight barrier
    fence(Ordering::SeqCst); // normal barrier
    membarrier2::heavy();     // heavy-weight barrier
}
