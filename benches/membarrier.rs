#![feature(test)]

extern crate test;

use test::Bencher;
use std::sync::atomic::{fence, Ordering};

#[bench]
fn light(b: &mut Bencher) {
    b.iter(|| {
        membarrier2::light();
    });
}

#[bench]
fn normal(b: &mut Bencher) {
    b.iter(|| {
        fence(Ordering::SeqCst);
    });
}

#[bench]
fn heavy(b: &mut Bencher) {
    b.iter(|| {
        membarrier2::heavy();
    });
}
