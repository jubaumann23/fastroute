//! Loader behaviour on the synthetic fixtures (no Java needed).

use fr_io::{load, load_bytes, InsertRequest, LoadError};

fn kinds(d: &fr_io::LoadedDesign) -> Vec<&'static str> {
    d.requests.iter().map(|r| r.kind_name()).collect()
}

#[test]
fn scope_order_matters() {
    let src = include_bytes!("../testdata/dsn/synth_order.dsn");
    let d = load_bytes(src).unwrap();
    // placement after network: no components are inserted
    assert_eq!(d.components.count(), 0);
    assert!(d.flip_style_rotate_first);
    let k = kinds(&d);
    assert_eq!(k[0], "BoardOutline");
    // hole of the outline on all 4 layers, 2 signal keepouts x 4 layers, 1 layer keepout
    assert_eq!(&k[1..14], &["ObstacleArea"; 13]);
    assert_eq!(
        &k[14..],
        &["Via", "PolylineTrace", "Via", "PolylineTrace", "Via"]
    );
    // the last via duplicates the second one
    assert!(matches!(
        d.requests.last().unwrap(),
        InsertRequest::Via {
            predicted_duplicate: true,
            ..
        }
    ));
    // canonical order: components are inserted, wires before vias
    let dsn = fr_dsn::Dsn::parse(src).unwrap();
    let c = load(&dsn).unwrap();
    assert_eq!(c.components.count(), 2);
    assert!(!c.flip_style_rotate_first);
}

#[test]
fn errors() {
    let src = include_bytes!("../testdata/dsn/synth_pcb_keepout.dsn");
    assert!(matches!(load_bytes(src), Err(LoadError::ParseError(_))));
    let no_outline = b"(pcb x (structure (layer A (type signal))))";
    assert!(matches!(
        load_bytes(no_outline),
        Err(LoadError::OutlineMissing(_))
    ));
    let no_layers = b"(pcb x (structure (boundary (rect pcb 0 0 10 10))))";
    assert!(matches!(
        load_bytes(no_layers),
        Err(LoadError::ParseError(_))
    ));
    // polyline_path outline: Java throws (null shape)
    let pl = b"(pcb x (structure (layer A (type signal)) (boundary (rect pcb 0 0 10 10)) \
               (boundary (polyline_path signal 0 0 0 10 0 10 0 10 10))))";
    assert!(matches!(load_bytes(pl), Err(LoadError::JavaException(_))));
}

#[test]
fn rules_and_nets() {
    let src = include_bytes!("../testdata/dsn/synth_rules.dsn");
    let d = load_bytes(src).unwrap();
    let r = &d.rules;
    // SIG2 has two fromto subnets, "Net-(R3-2)" starts at subnet 3
    let sig2: Vec<i32> = r
        .nets
        .get_all_by_name("SIG2")
        .iter()
        .map(|n| n.subnet_number)
        .collect();
    assert_eq!(sig2, vec![1, 2]);
    // nets 1 (VCC plane) and 2 (GND, missing power plane) come from the structure scope
    assert_eq!(r.nets.get_by_name("Net-(R3-2)", 3).unwrap().net_number, 8);
    // the ordered net SIG1 with 4 pins becomes 3 subnets
    assert_eq!(r.nets.get_all_by_name("SIG1").len(), 3);
    // Wiring via without net index increment bug: nets [last, 0]
    let via_nets: Vec<Vec<i32>> = d
        .requests
        .iter()
        .filter_map(|q| match q {
            InsertRequest::Via { nets, .. } => Some(nets.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(via_nets[2], vec![7, 0]);
    // a power layer exists: adjustPlaneAutorouteSettings returns immediately
    assert!(d.plane_adjustment.is_none());
    // degenerate x2, outside bounds, unknown layer, duplicate via
    assert_eq!(
        d.warnings.iter().filter(|w| w.contains("Wiring")).count(),
        5
    );
}
