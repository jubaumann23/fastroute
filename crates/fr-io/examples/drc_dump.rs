//! Lists the clearance violations of a DSN board as loaded (before routing).
//! Usage: cargo run --release -p fr-io --example drc_dump -- board.dsn
use fr_engine::board::{BasicBoard, ItemKey, ItemKind};

fn describe(board: &BasicBoard, key: ItemKey) -> String {
    let item = board.item(key);
    let kind = match &item.kind {
        ItemKind::Pin(_) => "pin",
        ItemKind::Via(_) => "via",
        ItemKind::Trace(_) => "trace",
        ItemKind::ObstacleArea(_) => "keepout",
        ItemKind::ConductionArea(_) => "conduction-area",
        ItemKind::ComponentOutline(_) => "outline",
        ItemKind::BoardOutline(_) => "board-outline",
    };
    let nets: Vec<String> = item
        .net_numbers()
        .iter()
        .map(|n| board.rules.nets.get(*n).map(|n| n.name.clone()).unwrap_or_default())
        .collect();
    let comp = if item.component_no() > 0 {
        board.components.get(item.component_no()).name.clone()
    } else {
        String::new()
    };
    format!("{kind}#{} [{}] {} {:?} cl={}", item.id().0, nets.join(","), comp, item.fixed_state(), item.clearance_class())
}

fn main() {
    let path = std::env::args().nth(1).expect("board.dsn");
    let src = std::fs::read(&path).unwrap();
    let mut settings = fr_settings::default_settings(1);
    let board = fr_io::post_load::load_from_specctra_dsn(&src, &mut settings).expect("load");
    let res = board.communication.resolution as f64;
    let violations = fr_engine::drc::all_clearance_violations(&board);
    println!("{} violations", violations.len());
    for v in &violations {
        let b = v.shape.bounding_box();
        println!(
            "L{} expected {:.0} actual {:.0} at ({:.1}, {:.1}) um\n    {}\n    {}",
            v.layer,
            v.expected_clearance / res,
            v.actual_clearance / res,
            (b.ll.x + b.ur.x) as f64 / 2.0 / res,
            (b.ll.y + b.ur.y) as f64 / 2.0 / res,
            describe(&board, v.first_item),
            describe(&board, v.second_item)
        );
    }
}
