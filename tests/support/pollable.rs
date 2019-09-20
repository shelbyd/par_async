use core::future::Future;
use futures_test::task::noop_context;
use std::{pin::Pin, task::Poll};

pub struct Pollable<F>(Pin<Box<F>>);

impl<F: Future> Pollable<F> {
    pub fn new(f: F) -> Self {
        Pollable(Box::pin(f))
    }

    pub fn poll(&mut self) -> Poll<F::Output> {
        let mut cx = noop_context();
        self.0.as_mut().poll(&mut cx)
    }
}
