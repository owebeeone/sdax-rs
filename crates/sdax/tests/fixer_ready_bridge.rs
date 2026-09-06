use sdax::host::{bodies_of, InstanceId};
use sdax::prelude::*;
use std::sync::Arc;

#[test]
fn join_publication_materializes_unit() {
    let mut p = Plan::builder("root");
    let join = p.join("joined", ());
    let plan = p
        .export(join)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let source = bodies_of(&plan);
    assert!(source.export().is_none());
    source.publish_ready(join.raw(), None).unwrap();
    assert!(source.export().unwrap().downcast::<Arc<()>>().is_ok());
    assert!(source
        .publish_ready(join.raw(), Some(InstanceId(999)))
        .is_err());
}

#[test]
fn component_publication_clones_real_export_and_refuses_missing_or_wrong_type() {
    let mut child = Plan::builder("child");
    let value = child.step("value").run(|_, ()| async { Ok(99_u32) });
    let child = child
        .export(value)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let mut root = Plan::builder("root");
    let component = root.component("child", &child);
    let root = root
        .export(component)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let source = bodies_of(&root);
    assert!(source.publish_ready(component.raw(), None).is_err());
    source.store(value.raw(), None, Box::new(Arc::new(())));
    assert!(source.publish_ready(component.raw(), None).is_err());
    let actual = Arc::new(99_u32);
    source.store(value.raw(), None, Box::new(actual.clone()));
    source.publish_ready(component.raw(), None).unwrap();
    let exported = source.export().unwrap().downcast::<Arc<u32>>().unwrap();
    assert!(Arc::ptr_eq(&actual, &exported));
    assert!(source.publish_ready(value.raw(), None).is_err());
    assert!(bodies_of(&root).export().is_none());
}

#[test]
fn explicit_unit_export_is_required_but_unexported_component_gets_unit() {
    let mut child = Plan::builder("child");
    let unit = child.join("unit", ());
    let child = child
        .export(unit)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let mut root = Plan::builder("root");
    let component = root.component("child", &child);
    let root = root
        .export(component)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let source = bodies_of(&root);
    assert!(source.publish_ready(component.raw(), None).is_err());
    source.publish_ready(unit.raw(), None).unwrap();
    source.publish_ready(component.raw(), None).unwrap();
    assert!(source.export().unwrap().downcast::<Arc<()>>().is_ok());

    let mut empty = Plan::builder("empty");
    empty.join("unit", ());
    let child = empty
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let mut root = Plan::builder("root");
    let component = root.component("empty", &child);
    let root = root
        .export(component)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let source = bodies_of(&root);
    source.publish_ready(component.raw(), None).unwrap();
    assert!(source.export().unwrap().downcast::<Arc<()>>().is_ok());
}

#[test]
fn component_can_export_imported_parent_value() {
    let mut root = Plan::builder("root");
    let value = root.step("value").run(|_, ()| async { Ok(99_u32) });
    let mut child = Plan::builder("child");
    let imported = child.import(value);
    child.join("ready", imported);
    let child = child
        .export(imported)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let component = root.component("child", &child);
    let root = root
        .export(component)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let source = bodies_of(&root);
    let actual = Arc::new(41_u32);
    source.store(value.raw(), None, Box::new(actual.clone()));
    source.publish_ready(component.raw(), None).unwrap();
    let exported = source.export().unwrap().downcast::<Arc<u32>>().unwrap();
    assert!(Arc::ptr_eq(&actual, &exported));
}

#[test]
fn instance_publication_never_reads_another_instance_or_closed_scope() {
    let mut child = Plan::builder("child");
    let value = child.step("value").run(|_, ()| async { Ok(99_u32) });
    let child = child
        .export(value)
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let mut template = Plan::with_input::<()>("template");
    let component = template.component("child", &child);
    let template = template
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .unwrap();
    let mut root = Plan::builder("root");
    let template = root.template("template", &template);
    let root = root
        .build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Resident,
        )
        .unwrap();
    let source = bodies_of(&root);
    let a = InstanceId(1);
    let b = InstanceId(2);
    source.open_instance(template.node(), None, a, Box::new(Arc::new(())));
    source.open_instance(template.node(), Some(a), b, Box::new(Arc::new(())));
    source.store(value.raw(), Some(a), Box::new(Arc::new(10_u32)));
    source.publish_ready(component.raw(), Some(a)).unwrap();
    assert!(source.publish_ready(component.raw(), Some(b)).is_err());
    source.store(value.raw(), Some(b), Box::new(Arc::new(20_u32)));
    source.publish_ready(component.raw(), Some(b)).unwrap();
    source.close_instance(b);
    assert!(source.publish_ready(component.raw(), Some(b)).is_err());
    assert!(source.publish_ready(component.raw(), None).is_err());
}
