use crate::support::pollable::*;
use core::future::Future;
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
}

impl Future for EchoFuture {
    type Output = String;
    fn poll(mut self: Pin<&mut Self>, _: &mut Context) -> Poll<Self::Output> {
        if self.returned {
            panic!("Polled previously resolved future");
        }

        let borrow = || self.shared.borrow_mut();

        *borrow().polls.entry(self.input.clone()).or_insert(0) += 1;

        if borrow().returns.contains(&self.input) {
            borrow().returns.remove(&self.input);
            self.returned = true;

            Poll::Ready(self.input.to_string())
        } else {
            Poll::Pending
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
}
