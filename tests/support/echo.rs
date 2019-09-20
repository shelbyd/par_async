use crate::support::pollable::*;
use core::future::Future;
use futures_test::task::noop_context;
use std::collections::{HashMap, HashSet};
use std::{
    cell::RefCell,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

#[derive(Default)]
struct SharedEcho {
    returns: HashSet<String>,
    requests: HashSet<String>,
    polls: HashMap<String, usize>,
}

#[derive(Default)]
pub struct Echo {
    shared: Rc<RefCell<SharedEcho>>,
}

impl Echo {
    pub fn new() -> Echo {
        Echo::default()
    }

    pub fn do_return(&self, s: &str) {
        let new_value = self.shared.borrow_mut().returns.insert(s.to_string());
        assert!(new_value);
        self.shared.borrow_mut().requests.remove(s);
    }

    pub fn echo(&self, s: &str) -> EchoFuture {
        if !self.shared.borrow().requests.contains(s) {
            let only_one_outstanding = self.shared.borrow_mut().requests.insert(s.to_string());
            assert!(only_one_outstanding);
        }

        EchoFuture {
            shared: self.shared.clone(),
            input: s.to_string(),
            returned: false,
            _pinned: core::marker::PhantomPinned,
            first_poll_location: None,
        }
    }

    pub fn outstanding_requests(&self) -> HashSet<String> {
        self.shared
            .borrow()
            .requests
            .difference(&self.shared.borrow().returns)
            .cloned()
            .collect()
    }

    pub fn polls(&self, s: &str) -> usize {
        self.shared.borrow().polls.get(s).cloned().unwrap_or(0)
    }
}

pub struct EchoFuture {
    shared: Rc<RefCell<SharedEcho>>,
    input: String,
    returned: bool,
    _pinned: core::marker::PhantomPinned,
    first_poll_location: Option<*const EchoFuture>,
}

impl Future for EchoFuture {
    type Output = String;
    fn poll(self: Pin<&mut Self>, _: &mut Context) -> Poll<Self::Output> {
        if self.returned {
            panic!("Polled previously resolved future");
        }

        let this = unsafe { self.get_unchecked_mut() };

        let im_here = this as *const EchoFuture;
        match this.first_poll_location {
            Some(ptr) if im_here != ptr => panic!("Future moved between polls"),
            None => this.first_poll_location = Some(im_here),
            _ => {}
        }

        let borrow = || this.shared.borrow_mut();

        *borrow().polls.entry(this.input.clone()).or_insert(0) += 1;

        if borrow().returns.contains(&this.input) {
            borrow().returns.remove(&this.input);
            this.returned = true;

            Poll::Ready(this.input.to_string())
        } else {
            Poll::Pending
        }
    }
}

// Extremely sensitive implementation of Drop that panics if the future was moved after poll was
// called.
impl Drop for EchoFuture {
    fn drop(&mut self) {
        if let Some(ptr) = self.first_poll_location {
            if ptr != self {
                if !::std::thread::panicking() {
                    panic!("Future moved before drop");
                }
            }
        }
    }
}

#[cfg(test)]
mod echo_tests {
    use super::*;

    #[test]
    fn echo_returns_immediately_when_previously_told_to() {
        let e = Echo::new();
        e.do_return("foo");

        let mut p = Pollable::new(e.echo("foo"));

        assert_eq!(p.poll(), Poll::Ready("foo".to_string()));
    }

    #[test]
    fn echo_pending_while_not_returning() {
        let e = Echo::new();

        let mut p = Pollable::new(e.echo("foo"));

        assert_eq!(p.poll(), Poll::Pending);
    }

    #[test]
    fn echo_ready_when_return_after_creation() {
        let e = Echo::new();

        let mut p = Pollable::new(e.echo("foo"));
        e.do_return("foo");

        assert_eq!(p.poll(), Poll::Ready("foo".to_string()));
    }

    #[test]
    fn echo_only_returns_from_first_poll_of_same_value() {
        let e = Echo::new();
        e.do_return("foo");

        let mut p = Pollable::new(e.echo("foo"));
        assert_eq!(p.poll(), Poll::Ready("foo".to_string()));

        let mut p = Pollable::new(e.echo("foo"));
        assert_eq!(p.poll(), Poll::Pending);
    }

    // Polling a resolved future violates the contract of the Future trait. This test ensures our
    // code would panic if that contract is violated.
    #[test]
    #[should_panic(expected = "Polled previously resolved future")]
    fn echo_polling_same_one_after_complete_panics() {
        let e = Echo::new();
        e.do_return("foo");

        let mut p = Pollable::new(e.echo("foo"));
        let _ = p.poll();

        let _ = p.poll(); // Panic here.
    }

    // Moving a pinned value violates the contract of Pin. This test ensures that our code would
    // panic if that contract is violated.
    #[test]
    #[should_panic(expected = "Future moved between polls")]
    fn echo_move_after_poll_panics() {
        let e = Echo::new();
        let mut fut = e.echo("foo");

        let mut cx = noop_context();
        let _ = unsafe { Pin::new_unchecked(&mut fut) }.poll(&mut cx);

        let mut moved_fut = fut;
        let _ = unsafe { Pin::new_unchecked(&mut moved_fut) }.poll(&mut cx); // Panic here.
    }

    // Moving a pinned value before it is dropped violates the contract of Pin. This test ensures
    // that our code would panic if that contract is violated.
    #[test]
    #[should_panic(expected = "Future moved before drop")]
    fn echo_move_after_poll_before_drop() {
        let e = Echo::new();
        let mut fut = e.echo("foo");

        let mut cx = noop_context();
        let _ = unsafe { Pin::new_unchecked(&mut fut) }.poll(&mut cx);

        let mut _moved_fut = fut;
    }
}
