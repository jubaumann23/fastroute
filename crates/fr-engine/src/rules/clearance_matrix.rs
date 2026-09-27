//! Port of `rules/ClearanceMatrix.java`: N x N matrix of the spacing restrictions between N
//! clearance classes, per layer.
//!
//! Class 0 is the "null" class (no clearance), class 1 the "default" class
//! (see [`ClearanceMatrix::get_default_instance`]).

use fr_jcompat::compare_to_ignore_case;

use crate::ids::{ClearanceClassNo, LayerNo};
use crate::structure::LayerStructure;

/// Java `ClearanceMatrix.clearance_safety_margin`.
pub const CLEARANCE_SAFETY_MARGIN: i32 = 16;

/// A row of the matrix (Java `ClearanceMatrix.Row`). `column[i][layer]` is the Java
/// `column[i].layer[layer]` (`MatrixEntry`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    name: String,
    column: Vec<Vec<i32>>,
    max_value: Vec<i32>,
}

impl Row {
    fn new(name: String, class_count: usize, layer_count: usize) -> Self {
        Row {
            name,
            column: vec![vec![0; layer_count]; class_count],
            max_value: vec![0; layer_count],
        }
    }
}

/// Clearance matrix (Java `ClearanceMatrix`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClearanceMatrix {
    layer_count: usize,
    /// Maximum clearance value for each layer.
    max_value_on_layer: Vec<i32>,
    row: Vec<Row>,
}

impl ClearanceMatrix {
    /// Creates a matrix for `class_count` (at least 1) classes; `names` needs one entry per
    /// class.
    pub fn new<S: AsRef<str>>(
        class_count: i32,
        layer_structure: &LayerStructure,
        names: &[S],
    ) -> Self {
        let class_count = class_count.max(1) as usize;
        let layer_count = layer_structure.layers.len();
        let row = (0..class_count)
            .map(|i| Row::new(names[i].as_ref().to_string(), class_count, layer_count))
            .collect();
        ClearanceMatrix {
            layer_count,
            max_value_on_layer: vec![0; layer_count],
            row,
        }
    }

    /// The matrix with the two classes "null" and "default", initialized with `default_value`.
    pub fn get_default_instance(layer_structure: &LayerStructure, default_value: i32) -> Self {
        let mut result = ClearanceMatrix::new(2, layer_structure, &["null", "default"]);
        result.set_default_value(default_value);
        result
    }

    /// The number of the class with the given name (ignoring case), or -1.
    pub fn get_no(&self, name: &str) -> ClearanceClassNo {
        self.row
            .iter()
            .position(|r| compare_to_ignore_case(&r.name, name) == 0)
            .map_or(-1, |i| i as ClearanceClassNo)
    }

    /// The name of the class with the given number; `None` (with a warning) if out of range.
    pub fn get_name(&self, clearance_class_index: ClearanceClassNo) -> Option<&str> {
        if clearance_class_index < 0 || clearance_class_index as usize >= self.row.len() {
            log::warn!("ClearanceMatrix.get_name: clearanceClassIndex out of range");
            return None;
        }
        Some(&self.row[clearance_class_index as usize].name)
    }

    /// Sets all entries between classes >= 1 to `value` on all layers.
    pub fn set_default_value(&mut self, value: i32) {
        for layer in 0..self.layer_count {
            self.set_default_value_on_layer(layer as LayerNo, value);
        }
    }

    /// Sets all entries between classes >= 1 to `value` on `layer`.
    pub fn set_default_value_on_layer(&mut self, layer: LayerNo, value: i32) {
        let n = self.class_count();
        for i in 1..n {
            for j in 1..n {
                self.set_value(i, j, layer, value);
            }
        }
    }

    /// Sets an entry to `value` on all layers.
    pub fn set_value_all_layers(
        &mut self,
        class_i: ClearanceClassNo,
        class_j: ClearanceClassNo,
        value: i32,
    ) {
        for layer in 0..self.layer_count {
            self.set_value(class_i, class_j, layer as LayerNo, value);
        }
    }

    /// Sets the entry (row `class_j`, column `class_i`) on `layer`. Negative values become 0,
    /// odd values are rounded up to even (`Integer.MAX_VALUE` down). The row and layer maxima
    /// never decrease.
    pub fn set_value(
        &mut self,
        class_i: ClearanceClassNo,
        class_j: ClearanceClassNo,
        layer: LayerNo,
        value: i32,
    ) {
        let mut value = value.max(0);
        if value % 2 != 0 {
            if value == i32::MAX {
                value -= 1;
            } else {
                value += 1;
            }
        }
        let layer = layer as usize;
        let current_row = &mut self.row[class_j as usize];
        current_row.column[class_i as usize][layer] = value;
        current_row.max_value[layer] = current_row.max_value[layer].max(value);
        self.max_value_on_layer[layer] = self.max_value_on_layer[layer].max(value);
    }

    /// Sets an entry to `value` on all inner layers.
    pub fn set_inner_value(
        &mut self,
        class_i: ClearanceClassNo,
        class_j: ClearanceClassNo,
        value: i32,
    ) {
        for layer in 1..self.layer_count.saturating_sub(1) {
            self.set_value(class_i, class_j, layer as LayerNo, value);
        }
    }

    /// The required spacing between classes `class_i` and `class_j` on `layer` (always even),
    /// plus [`CLEARANCE_SAFETY_MARGIN`] if requested. Out of range arguments give 0.
    pub fn get_value(
        &self,
        class_i: ClearanceClassNo,
        class_j: ClearanceClassNo,
        layer: LayerNo,
        add_safety_margin: bool,
    ) -> i32 {
        let n = self.class_count();
        if class_i < 0
            || class_i >= n
            || class_j < 0
            || class_j >= n
            || layer < 0
            || layer as usize >= self.layer_count
        {
            return 0;
        }
        let value = self.row[class_j as usize].column[class_i as usize][layer as usize];
        if add_safety_margin {
            value.wrapping_add(CLEARANCE_SAFETY_MARGIN)
        } else {
            value
        }
    }

    /// The maximal spacing of `class_i` to all other classes on `layer` (arguments clamped).
    pub fn max_value(&self, class_i: ClearanceClassNo, layer: LayerNo) -> i32 {
        let i = class_i.max(0).min(self.class_count() - 1);
        let layer_index = layer.max(0).min(self.layer_count as i32 - 1);
        self.row[i as usize].max_value[layer_index as usize]
    }

    /// The maximum clearance value on `layer` (clamped); Java `maxValue(int layer)`.
    pub fn max_value_on_layer(&self, layer: LayerNo) -> i32 {
        let layer_index = layer.max(0).min(self.layer_count as i32 - 1);
        self.max_value_on_layer[layer_index as usize]
    }

    /// True if the entry (column `class_i`, row `class_j`) differs between layers.
    pub fn is_layer_dependent(&self, class_i: ClearanceClassNo, class_j: ClearanceClassNo) -> bool {
        let entry = &self.row[class_j as usize].column[class_i as usize];
        let compare_value = entry[0];
        entry[1..].iter().any(|&v| v != compare_value)
    }

    /// True if the entry differs between inner layers.
    pub fn is_inner_layer_dependent(
        &self,
        class_i: ClearanceClassNo,
        class_j: ClearanceClassNo,
    ) -> bool {
        if self.layer_count <= 2 {
            return false;
        }
        let entry = &self.row[class_j as usize].column[class_i as usize];
        let compare_value = entry[1];
        entry[2..self.layer_count - 1]
            .iter()
            .any(|&v| v != compare_value)
    }

    /// The number of clearance classes.
    #[inline]
    pub fn get_class_count(&self) -> i32 {
        self.row.len() as i32
    }

    #[inline]
    fn class_count(&self) -> i32 {
        self.row.len() as i32
    }

    /// The layer count of this matrix.
    pub fn get_layer_count(&self) -> i32 {
        self.layer_count as i32
    }

    /// Clearance compensation value of a class on a layer: `(value(c, c, layer) + 1) / 2`.
    /// Used by the search trees to enlarge item shapes.
    pub fn clearance_compensation_value(
        &self,
        clearance_class_index: ClearanceClassNo,
        layer: LayerNo,
    ) -> i32 {
        self.get_value(clearance_class_index, clearance_class_index, layer, false)
            .wrapping_add(1)
            / 2
    }

    /// Appends a class initialized with the values of the default class (1). False if a class
    /// with that name (ignoring case) exists.
    pub fn append_class(&mut self, class_name: &str) -> bool {
        if self.get_no(class_name) >= 0 {
            return false;
        }
        let old_class_count = self.row.len();
        let layer_count = self.layer_count;
        // Java rebuilds the rows: old entries and max values are kept, the new column is 0.
        for r in &mut self.row {
            r.column.push(vec![0; layer_count]);
        }
        self.row.push(Row::new(
            class_name.to_string(),
            old_class_count + 1,
            layer_count,
        ));

        let new_class = old_class_count as ClearanceClassNo;
        for i in 0..old_class_count as ClearanceClassNo {
            for j in 0..layer_count as LayerNo {
                let default_value = self.get_value(1, i, j, false);
                self.set_value(new_class, i, j, default_value);
                self.set_value(i, new_class, j, default_value);
            }
        }
        for j in 0..layer_count as LayerNo {
            let default_value = self.get_value(1, 1, j, false);
            self.set_value(new_class, new_class, j, default_value);
        }
        true
    }

    /// Removes the class with the given index. Faithful to Java, the row maxima of the
    /// remaining rows are reset to 0 (Java creates new `Row`s without copying `maxValue`) and
    /// the per-layer maxima are kept.
    pub(crate) fn remove_class(&mut self, index: ClearanceClassNo) {
        let index = index as usize;
        let layer_count = self.layer_count;
        let old_rows = std::mem::take(&mut self.row);
        self.row = old_rows
            .into_iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .map(|(_, r)| Row {
                name: r.name,
                column: r
                    .column
                    .into_iter()
                    .enumerate()
                    .filter(|(j, _)| *j != index)
                    .map(|(_, c)| c)
                    .collect(),
                max_value: vec![0; layer_count],
            })
            .collect();
    }

    /// True if all values (columns 1..) of the classes `first` and `second` are equal.
    pub fn is_equal(&self, first: ClearanceClassNo, second: ClearanceClassNo) -> bool {
        if first == second {
            return true;
        }
        let n = self.class_count();
        if first < 0 || second < 0 || first >= n || second >= n {
            return false;
        }
        let row1 = &self.row[first as usize];
        let row2 = &self.row[second as usize];
        (1..n as usize).all(|i| row1.column[i] == row2.column[i])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structure::Layer;

    fn layers(n: usize) -> LayerStructure {
        LayerStructure::new((0..n).map(|i| Layer::new(format!("L{i}"), true)).collect())
    }

    /// Port of `ClearanceMatrixTest.setValue`.
    #[test]
    fn set_value_java_test() {
        let ls = LayerStructure::new(vec![Layer::new("Top", true), Layer::new("Bottom", true)]);
        let mut matrix = ClearanceMatrix::new(1, &ls, &["default"]);

        matrix.set_value(0, 0, 0, 5);
        assert_eq!(matrix.get_value(0, 0, 0, false), 6);
        assert_eq!(matrix.max_value(0, 0), 6);
        assert_eq!(matrix.max_value_on_layer(0), 6);

        matrix.set_value(0, 0, 0, -10);
        assert_eq!(matrix.get_value(0, 0, 0, false), 0);
        assert_eq!(matrix.max_value(0, 0), 6);
        assert_eq!(matrix.max_value_on_layer(0), 6);

        matrix.set_value(0, 0, 0, i32::MAX);
        assert_eq!(matrix.get_value(0, 0, 0, false), i32::MAX - 1);
        assert_eq!(matrix.max_value(0, 0), i32::MAX - 1);
        assert_eq!(matrix.max_value_on_layer(0), i32::MAX - 1);
        // Java int overflow of the safety margin
        assert_eq!(
            matrix.get_value(0, 0, 0, true),
            (i32::MAX - 1).wrapping_add(16)
        );
    }

    #[test]
    fn default_instance() {
        let m = ClearanceMatrix::get_default_instance(&layers(3), 201);
        assert_eq!(m.get_class_count(), 2);
        assert_eq!(m.get_layer_count(), 3);
        assert_eq!(m.get_name(0), Some("null"));
        assert_eq!(m.get_name(1), Some("default"));
        assert_eq!(m.get_name(2), None);
        assert_eq!(m.get_no("DEFAULT"), 1);
        assert_eq!(m.get_no("Null"), 0);
        assert_eq!(m.get_no("x"), -1);
        for l in 0..3 {
            assert_eq!(m.get_value(1, 1, l, false), 202);
            assert_eq!(m.get_value(0, 1, l, false), 0);
            assert_eq!(m.get_value(1, 0, l, false), 0);
            assert_eq!(m.get_value(1, 1, l, true), 218);
        }
        assert_eq!(m.get_value(2, 1, 0, false), 0);
        assert_eq!(m.get_value(1, 1, 3, false), 0);
        assert_eq!(m.get_value(-1, 1, 0, false), 0);
        assert_eq!(m.max_value(0, 0), 0);
        assert_eq!(m.max_value(1, 0), 202);
        assert_eq!(m.max_value(7, -3), 202);
        assert_eq!(m.max_value_on_layer(99), 202);
        assert_eq!(m.clearance_compensation_value(1, 0), 101);
        assert_eq!(m.clearance_compensation_value(0, 0), 0);
        assert_eq!(m.clearance_compensation_value(5, 0), 0);
    }

    #[test]
    fn per_layer_values() {
        let mut m = ClearanceMatrix::get_default_instance(&layers(4), 100);
        assert!(!m.is_layer_dependent(1, 1));
        m.set_inner_value(1, 1, 300);
        assert_eq!(m.get_value(1, 1, 0, false), 100);
        assert_eq!(m.get_value(1, 1, 1, false), 300);
        assert_eq!(m.get_value(1, 1, 2, false), 300);
        assert_eq!(m.get_value(1, 1, 3, false), 100);
        assert!(m.is_layer_dependent(1, 1));
        assert!(!m.is_inner_layer_dependent(1, 1));
        m.set_value(1, 1, 2, 51);
        assert!(m.is_inner_layer_dependent(1, 1));
        assert_eq!(m.get_value(1, 1, 2, false), 52);
        // maxima never decrease
        assert_eq!(m.max_value(1, 2), 300);
        assert_eq!(m.max_value_on_layer(2), 300);
        assert_eq!(m.clearance_compensation_value(1, 2), 26);
        m.set_default_value_on_layer(0, 11);
        assert_eq!(m.get_value(1, 1, 0, false), 12);
        assert_eq!(m.max_value(1, 0), 100);
        // only 2 layers: never inner layer dependent
        let two = ClearanceMatrix::get_default_instance(&layers(2), 10);
        assert!(!two.is_inner_layer_dependent(1, 1));
    }

    #[test]
    fn asymmetric_rows_and_columns() {
        let mut m = ClearanceMatrix::get_default_instance(&layers(2), 100);
        assert!(m.append_class("smd"));
        // set_value(i, j) writes row j, column i and updates the max of row j
        m.set_value(2, 1, 0, 500);
        assert_eq!(m.get_value(2, 1, 0, false), 500);
        assert_eq!(m.get_value(1, 2, 0, false), 100);
        assert_eq!(m.max_value(1, 0), 500);
        assert_eq!(m.max_value(2, 0), 100);
        assert_eq!(m.max_value_on_layer(0), 500);
        assert_eq!(m.max_value_on_layer(1), 100);
    }

    #[test]
    fn append_class() {
        let mut m = ClearanceMatrix::get_default_instance(&layers(2), 100);
        m.set_value(1, 1, 1, 150);
        assert!(m.append_class("power"));
        assert!(!m.append_class("POWER"));
        assert!(!m.append_class("Default"));
        assert_eq!(m.get_class_count(), 3);
        assert_eq!(m.get_no("power"), 2);
        // new entries copied from the default class, per layer
        for (l, v) in [(0, 100), (1, 150)] {
            assert_eq!(m.get_value(2, 2, l, false), v);
            assert_eq!(m.get_value(1, 2, l, false), v);
            assert_eq!(m.get_value(2, 1, l, false), v);
            // row/column of class 0: default(1, 0) is 0
            assert_eq!(m.get_value(2, 0, l, false), 0);
            assert_eq!(m.get_value(0, 2, l, false), 0);
        }
        assert_eq!(m.max_value(2, 1), 150);
        assert!(m.is_equal(1, 2));
        m.set_value(2, 2, 0, 40);
        assert!(!m.is_equal(1, 2));
        assert!(m.is_equal(2, 2));
        assert!(!m.is_equal(2, 3));
        assert!(!m.is_equal(-1, 2));
        // is_equal ignores column 0
        let mut m2 = ClearanceMatrix::get_default_instance(&layers(2), 100);
        m2.append_class("x");
        m2.set_value(0, 2, 0, 8);
        assert!(m2.is_equal(1, 2));
    }

    #[test]
    fn append_to_single_class_matrix() {
        // class 1 does not exist: get_value(1, ..) is 0
        let mut m = ClearanceMatrix::new(1, &layers(2), &["only"]);
        m.set_value(0, 0, 0, 20);
        assert!(m.append_class("b"));
        assert_eq!(m.get_value(0, 1, 0, false), 0);
        assert_eq!(m.get_value(1, 1, 0, false), 0);
        assert_eq!(m.get_value(0, 0, 0, false), 20);
    }

    #[test]
    fn remove_class() {
        let mut m = ClearanceMatrix::get_default_instance(&layers(2), 100);
        m.append_class("a");
        m.append_class("b");
        m.set_value(3, 3, 0, 300);
        m.set_value(2, 3, 0, 250);
        m.remove_class(2);
        assert_eq!(m.get_class_count(), 3);
        assert_eq!(m.get_name(2), Some("b"));
        assert_eq!(m.get_value(2, 2, 0, false), 300);
        assert_eq!(m.get_value(1, 2, 0, false), 100);
        // Java quirk: row maxima are reset, layer maxima kept
        assert_eq!(m.max_value(2, 0), 0);
        assert_eq!(m.max_value(1, 0), 0);
        assert_eq!(m.max_value_on_layer(0), 300);
    }
}
