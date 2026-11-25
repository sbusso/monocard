//! A sidecar as Monocard 0.1.2 writes it, at schema 10 before Greycard
//! had shipped that schema: Greycard's golden states as of 0cf4e83, and
//! two monochrome develops. The file is never written again. A merge that
//! fails here reads a released Monocard's edits differently, and is to
//! migrate them.
#![cfg(feature = "mono")]

use greycard_edit::{Edit, Sidecar};

const KEPT: &str = include_str!("../fixtures/monocard-0.1.2.mcd");

#[test]
fn an_edit_of_0_1_2_is_read_and_written_as_it_was() {
    let s = Sidecar::from_json(KEPT, &Edit::default()).unwrap();
    assert_eq!(s.to_json(), KEPT);
    let m = s.current().mono.as_deref().unwrap();
    assert_eq!(m.look.name, "Street");
    assert_eq!(m.basic.exposure, 0.7);
    assert_eq!(m.tone_eq.zones[3], -0.5);
    assert_eq!(m.curve.points[1], [0.4, 0.5]);
    assert_eq!(m.grain.amount, 0.4);
    assert_eq!(m.toning.overall.amount, 0.2);
}
