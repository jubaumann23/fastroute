//! Port of `core/library/LogicalPart.java` and `LogicalParts.java` (gate swap / pin swap
//! information).

use super::padstack::equals_ignore_case;
use super::LogicalPartNo;

/// A pin of a logical part (Java `LogicalPart.PartPin`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartPin {
    /// Index of the pin; the same as in the component's library package.
    pub pin_index: i32,
    /// Name of the pin; the same as in the component's library package.
    pub pin_name: String,
    pub gate_name: String,
    /// Gates with the same swap code can be swapped; `<= 0` is not swappable.
    pub gate_swap_code: i32,
    pub gate_pin_name: String,
    /// Pins with the same swap code can be swapped inside a gate; `<= 0` is not swappable.
    pub gate_pin_swap_code: i32,
}

impl PartPin {
    /// Java `compareTo`: `pinIndex - other.pinIndex` (int subtraction).
    pub fn compare_to(&self, other: &PartPin) -> i32 {
        self.pin_index.wrapping_sub(other.pin_index)
    }
}

/// Gate swap and pin swap information for a component (Java `LogicalPart`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogicalPart {
    pub name: String,
    /// 1-based id.
    pub id: LogicalPartNo,
    part_pin_arr: Vec<PartPin>,
}

impl LogicalPart {
    /// Creates a logical part; the part pins are expected sorted by pin index.
    pub fn new(name: impl Into<String>, id: LogicalPartNo, part_pin_arr: Vec<PartPin>) -> Self {
        LogicalPart {
            name: name.into(),
            id,
            part_pin_arr,
        }
    }

    pub fn pin_count(&self) -> i32 {
        self.part_pin_arr.len() as i32
    }

    /// The pin with the given index; `None` (with a warning) if out of range.
    pub fn get_pin(&self, pin_index: i32) -> Option<&PartPin> {
        if pin_index < 0 || pin_index as usize >= self.part_pin_arr.len() {
            log::warn!("LogicalPart.getPin: pinIndex out of range");
            return None;
        }
        Some(&self.part_pin_arr[pin_index as usize])
    }
}

/// The logical parts of the board (Java `LogicalParts`). Ids are 1-based.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogicalParts {
    part_arr: Vec<LogicalPart>,
}

impl LogicalParts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a logical part (pins sorted stably by pin index); returns its id.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        mut part_pin_arr: Vec<PartPin>,
    ) -> LogicalPartNo {
        part_pin_arr.sort_by(|a, b| a.compare_to(b).cmp(&0));
        let id = self.part_arr.len() as LogicalPartNo + 1;
        self.part_arr.push(LogicalPart::new(name, id, part_pin_arr));
        id
    }

    /// The logical part with the given name (ignoring case), if any.
    pub fn get_by_name(&self, name: &str) -> Option<&LogicalPart> {
        self.part_arr
            .iter()
            .find(|p| equals_ignore_case(&p.name, name))
    }

    /// The logical part with the given id (1-based). Panics if out of range (Java
    /// `ArrayIndexOutOfBoundsException`).
    pub fn get(&self, part_id: LogicalPartNo) -> &LogicalPart {
        let result = &self.part_arr[(part_id - 1) as usize];
        if result.id != part_id {
            log::warn!("LogicalParts.get: inconsistent part ID");
        }
        result
    }

    pub fn count(&self) -> i32 {
        self.part_arr.len() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pp(i: i32, name: &str) -> PartPin {
        PartPin {
            pin_index: i,
            pin_name: name.into(),
            gate_name: "G".into(),
            gate_swap_code: 1,
            gate_pin_name: name.into(),
            gate_pin_swap_code: 0,
        }
    }

    #[test]
    fn add_sorts_pins() {
        let mut lp = LogicalParts::new();
        let id = lp.add(
            "Nand",
            vec![pp(2, "c"), pp(0, "a"), pp(1, "b"), pp(0, "a2")],
        );
        assert_eq!(id, 1);
        let p = lp.get(1);
        let names: Vec<_> = (0..p.pin_count())
            .map(|i| p.get_pin(i).unwrap().pin_name.as_str())
            .collect();
        assert_eq!(names, ["a", "a2", "b", "c"]);
        assert!(p.get_pin(4).is_none());
        assert_eq!(lp.get_by_name("NAND").unwrap().id, 1);
        assert!(lp.get_by_name("nor").is_none());
        assert_eq!(lp.count(), 1);
    }
}
