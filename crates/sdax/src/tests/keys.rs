//! Keys and dependency tuples (witness A1/A4 of `B/experiments/typed_keys`).

use crate::host::{RawKey, Slots};
use crate::*;
use std::sync::Arc;

pub trait Endpoint: Send + Sync {
    fn addr(&self) -> String;
}
struct Quic;
impl Endpoint for Quic {
    fn addr(&self) -> String {
        "127.0.0.1:9000".into()
    }
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn keys_are_copy_send_sync_for_sized_and_unsized_types() {
    assert_send_sync::<Key<u8>>();
    assert_send_sync::<Key<dyn Endpoint>>();
    fn assert_copy<T: Copy>() {}
    assert_copy::<Key<u8>>();
    assert_copy::<Key<dyn Endpoint>>();
}

#[test]
fn deps_map_key_tuples_to_arc_tuples_up_to_eight() {
    // The `Deps::Out` associated type is what a body receives.
    fn out_of<D: Deps>(_d: D) -> std::marker::PhantomData<D::Out> {
        std::marker::PhantomData
    }
    let _: std::marker::PhantomData<()> = out_of(());
    let k: Key<u8> = Key::from_raw(RawKey { plan: 1, idx: 0 });
    let _: std::marker::PhantomData<Arc<u8>> = out_of(k);
    let _: std::marker::PhantomData<(Arc<u8>,)> = out_of((k,));
    let _: std::marker::PhantomData<(Arc<u8>, Arc<u8>)> = out_of((k, k));
    type Eight = (
        Arc<u8>,
        Arc<u8>,
        Arc<u8>,
        Arc<u8>,
        Arc<u8>,
        Arc<u8>,
        Arc<u8>,
        Arc<u8>,
    );
    let _: std::marker::PhantomData<Eight> = out_of((k, k, k, k, k, k, k, k));
}

#[test]
fn deps_report_their_raw_keys_in_declaration_order() {
    let a: Key<u8> = Key::from_raw(RawKey { plan: 7, idx: 0 });
    let b: Key<u16> = Key::from_raw(RawKey { plan: 7, idx: 1 });
    let c: Key<u32> = Key::from_raw(RawKey { plan: 7, idx: 2 });
    assert_eq!(().raw_keys(), Vec::<RawKey>::new());
    assert_eq!(a.raw_keys(), vec![a.raw()]);
    assert_eq!((a, b, c).raw_keys(), vec![a.raw(), b.raw(), c.raw()]);
}

#[test]
fn slots_carry_unsized_outputs_and_report_an_empty_slot_as_none() {
    let key: Key<dyn Endpoint> = Key::from_raw(RawKey { plan: 1, idx: 0 });
    let mut slots = Slots::new(2);
    assert!(slots.get::<dyn Endpoint>(key.raw()).is_none());
    let ep: Arc<dyn Endpoint> = Arc::new(Quic);
    slots.set(key.raw(), ep);
    assert_eq!(
        slots.get::<dyn Endpoint>(key.raw()).unwrap().addr(),
        "127.0.0.1:9000"
    );
}
