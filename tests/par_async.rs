use core::future::Future;
use futures_test::task::noop_context;
use futures_util::future::FutureExt;
use par_async::*;
use pin_utils::pin_mut;
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
struct Echo {
    shared: Rc<RefCell<SharedEcho>>,
}

impl Echo {
    fn new() -> Echo {
        Echo::default()
    }

    fn do_return(&self, s: &str) {
        let new_value = self.shared.borrow_mut().returns.insert(s.to_string());
        assert!(new_value);
        self.shared.borrow_mut().requests.remove(s);
    }

    fn echo(&self, s: &str) -> EchoFuture {
        if !self.shared.borrow().requests.contains(s) {
            let only_one_outstanding = self.shared.borrow_mut().requests.insert(s.to_string());
            assert!(only_one_outstanding);
        }

        EchoFuture {
            shared: self.shared.clone(),
            input: s.to_string(),
        }
    }

    fn outstanding_requests(&self) -> HashSet<String> {
        self.shared
            .borrow()
            .requests
            .difference(&self.shared.borrow().returns)
            .cloned()
            .collect()
    }

    fn polls(&self, s: &str) -> usize {
        self.shared.borrow().polls.get(s).cloned().unwrap_or(0)
    }
}

struct EchoFuture {
    shared: Rc<RefCell<SharedEcho>>,
    input: String,
}

impl Future for EchoFuture {
    type Output = String;
    fn poll(self: Pin<&mut Self>, _: &mut Context) -> Poll<Self::Output> {
        {
            let mut borrow = self.shared.borrow_mut();
            let count = borrow.polls.entry(self.input.clone()).or_insert(0);
            *count += 1;
        }

        if self.shared.borrow().returns.contains(&self.input) {
            self.shared.borrow_mut().returns.remove(&self.input);
            Poll::Ready(self.input.to_string())
        } else {
            Poll::Pending
        }
    }
}

fn hash_set(strings: &[&str]) -> HashSet<String> {
    strings.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod echo_tests {
    use super::*;

    #[test]
    fn echo_returns_immediately_when_previously_told_to() {
        let e = Echo::new();
        e.do_return("foo");

        let future = e.echo("foo");

        pin_mut!(future);
        let mut cx = noop_context();
        assert_eq!(future.poll_unpin(&mut cx), Poll::Ready("foo".to_string()));
    }

    #[test]
    fn echo_pending_while_not_returning() {
        let e = Echo::new();

        let future = e.echo("foo");

        pin_mut!(future);
        let mut cx = noop_context();
        assert_eq!(future.poll_unpin(&mut cx), Poll::Pending);
    }

    #[test]
    fn echo_ready_when_return_after_creation() {
        let e = Echo::new();

        let future = e.echo("foo");
        e.do_return("foo");

        pin_mut!(future);
        let mut cx = noop_context();
        assert_eq!(future.poll_unpin(&mut cx), Poll::Ready("foo".to_string()));
    }

    #[test]
    fn echo_only_returns_from_first_poll_of_same_value() {
        let e = Echo::new();
        e.do_return("foo");

        let future = e.echo("foo");
        pin_mut!(future);
        let mut cx = noop_context();
        assert_eq!(future.poll_unpin(&mut cx), Poll::Ready("foo".to_string()));

        let future = e.echo("foo");
        pin_mut!(future);
        let mut cx = noop_context();
        assert_eq!(future.poll_unpin(&mut cx), Poll::Pending);
    }
}

#[test]
fn it_does_not_affect_non_async_functions() {
    #[par_async]
    fn foo() -> u32 {
        42
    }

    assert_eq!(foo(), 42);
}

#[test]
fn it_immediately_returns_immediate_async() {
    #[par_async]
    async fn foo() -> u32 {
        42
    }

    let f = foo();
    pin_mut!(f);
    let mut cx = noop_context();
    assert_eq!(f.poll_unpin(&mut cx), Poll::Ready(42));
}

#[test]
fn it_awaits_a_future() {
    #[par_async]
    async fn foo(e: &Echo) -> String {
        e.echo("foo").await
    }

    let e = Echo::new();
    e.do_return("foo");

    let f = foo(&e);
    pin_mut!(f);
    let mut cx = noop_context();
    assert_eq!(f.poll_unpin(&mut cx), Poll::Ready("foo".to_string()));
}

#[test]
fn it_parallelizes_two_futures() {
    #[par_async]
    async fn foo(e: &Echo) -> String {
        let foo = e.echo("foo").await;
        let bar = e.echo("bar").await;
        foo + &bar
    }

    let e = Echo::new();

    let f = foo(&e);
    pin_mut!(f);
    let mut cx = noop_context();
    assert_eq!(f.poll_unpin(&mut cx), Poll::Pending);

    assert_eq!(e.outstanding_requests(), hash_set(&["foo", "bar"]));

    assert_eq!(e.polls("foo"), 1);
    assert_eq!(e.polls("bar"), 1);

    e.do_return("foo");
    e.do_return("bar");

    assert_eq!(f.poll_unpin(&mut cx), Poll::Ready("foobar".to_string()));
}

#[test]
fn it_parallelizes_three_futures() {
    #[par_async]
    async fn foo(e: &Echo) -> String {
        let foo = e.echo("foo").await;
        let bar = e.echo("bar").await;
        let baz = e.echo("baz").await;
        foo + &bar + &baz
    }

    let e = Echo::new();

    let f = foo(&e);
    pin_mut!(f);
    let mut cx = noop_context();
    assert_eq!(f.poll_unpin(&mut cx), Poll::Pending);

    assert_eq!(e.outstanding_requests(), hash_set(&["foo", "bar", "baz"]));

    assert_eq!(e.polls("foo"), 1);
    assert_eq!(e.polls("bar"), 1);
    assert_eq!(e.polls("baz"), 1);

    e.do_return("foo");
    e.do_return("bar");
    e.do_return("baz");

    assert_eq!(f.poll_unpin(&mut cx), Poll::Ready("foobarbaz".to_string()));
}
