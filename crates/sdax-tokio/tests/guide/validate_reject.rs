use sdax::prelude::*;
use std::sync::Arc;
use std::time::Duration;

struct Conn;

#[test]
fn build_refuses_and_the_finding_names_the_node() {
    let empty = Plan::builder("Empty")
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect_err("empty");
    assert_eq!(empty.checks.len(), 1);
    assert_eq!(empty.checks[0].rule, Rule::Empty);

    let mut p = Plan::builder("Twice");
    let conn = p
        .resource("Conn")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Conn)) })
        .release(|_cx, _c: Arc<Conn>| async move { Ok(()) });
    p.step("Ping")
        .needs(conn)
        .run(|_cx, _c: Arc<Conn>| async move { Ok(()) });
    p.step("Ping")
        .needs(conn)
        .run(|_cx, _c: Arc<Conn>| async move { Ok(()) });
    let err = p
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect_err("two nodes named Ping");
    let dup = err
        .checks
        .iter()
        .find(|f| f.rule == Rule::DupName)
        .expect("V-DUP-NAME");
    assert_eq!(dup.nodes, vec!["Ping".to_string(), "Ping".to_string()]);
    assert!(dup.detail.contains("Ping"), "{}", dup.detail);
    assert!(!dup.fix.is_empty());
}
