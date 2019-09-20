use par_async::*;
use std::collections::HashSet;
use std::task::Poll;

mod support;
use self::support::*;

fn hash_set(strings: &[&str]) -> HashSet<String> {
    strings.iter().map(|s| s.to_string()).collect()
}

fn assert_outstanding(e: &Echo, strs: &[&str]) {
    assert_eq!(e.outstanding_requests(), hash_set(strs));

    for s in strs {
        assert!(e.polls(s) > 0);
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

    let mut p = Pollable::new(foo());
    assert_eq!(p.poll(), Poll::Ready(42));
}

#[test]
fn it_awaits_a_future() {
    #[par_async]
    async fn foo(e: &Echo) -> String {
        e.echo("foo").await
    }

    let e = Echo::new();
    e.do_return("foo");

    let mut p = Pollable::new(foo(&e));
    assert_eq!(p.poll(), Poll::Ready("foo".to_string()));
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
    let mut p = Pollable::new(foo(&e));

    assert_eq!(p.poll(), Poll::Pending);
    assert_outstanding(&e, &["foo", "bar"]);

    e.do_return("foo");
    e.do_return("bar");

    assert_eq!(p.poll(), Poll::Ready("foobar".to_string()));
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
    let mut p = Pollable::new(foo(&e));

    assert_eq!(p.poll(), Poll::Pending);
    assert_outstanding(&e, &["foo", "bar", "baz"]);

    e.do_return("foo");
    e.do_return("bar");
    e.do_return("baz");

    assert_eq!(p.poll(), Poll::Ready("foobarbaz".to_string()));
}

#[test]
fn it_allows_expressions_of_the_await() {
    #[par_async]
    async fn foo(e: &Echo) -> String {
        let foo = e.echo("foo").await + "more";
        let bar = e.echo("bar").await + "more";
        foo + &bar
    }

    let e = Echo::new();
    let mut p = Pollable::new(foo(&e));

    assert_eq!(p.poll(), Poll::Pending);
    assert_outstanding(&e, &["foo", "bar"]);

    e.do_return("foo");
    e.do_return("bar");

    assert_eq!(p.poll(), Poll::Ready("foomorebarmore".to_string()));
}
