//! Port of `board/model/structure/Layer.java` and `LayerStructure.java`.

use crate::ids::LayerNo;

/// Describes the structure of a board layer (Java `Layer`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layer {
    /// The name of the layer.
    pub name: String,
    /// True, if this is a signal layer usable for routing (otherwise e.g. a power plane).
    pub is_signal: bool,
}

impl Layer {
    pub fn new(name: impl Into<String>, is_signal: bool) -> Self {
        Layer {
            name: name.into(),
            is_signal,
        }
    }
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// Describes the layer structure of the board (Java `LayerStructure`).
///
/// Java compares `Layer` objects by identity in `getNo(Layer)`/`getSignalLayerNo(Layer)`;
/// here layers are identified by their index instead.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct LayerStructure {
    pub layers: Vec<Layer>,
}

impl LayerStructure {
    pub fn new(layers: Vec<Layer>) -> Self {
        LayerStructure { layers }
    }

    /// Number of layers (`layers.length`).
    #[inline]
    pub fn layer_count(&self) -> i32 {
        self.layers.len() as i32
    }

    /// Index of the layer with the given name (case-sensitive), or -1.
    pub fn get_no(&self, name: &str) -> LayerNo {
        self.layers
            .iter()
            .position(|l| l.name == name)
            .map_or(-1, |i| i as LayerNo)
    }

    /// Returns the count of signal layers.
    pub fn signal_layer_count(&self) -> i32 {
        self.layers.iter().filter(|l| l.is_signal).count() as i32
    }

    /// Index of the `no`-th signal layer; the last layer if there are not enough signal layers
    /// (Java `getSignalLayer` returns that layer object). Panics if there are no layers.
    pub fn get_signal_layer_index(&self, no: i32) -> usize {
        let mut found = 0;
        for (i, layer) in self.layers.iter().enumerate() {
            if layer.is_signal {
                if no == found {
                    return i;
                }
                found += 1;
            }
        }
        self.layers.len() - 1
    }

    /// Gets the `no`-th signal layer (Java `getSignalLayer`).
    pub fn get_signal_layer(&self, no: i32) -> &Layer {
        &self.layers[self.get_signal_layer_index(no)]
    }

    /// Returns the count of signal layers with a smaller index than `layer`, or -1 if `layer`
    /// is not a valid index (Java `getSignalLayerNo(Layer)`).
    pub fn get_signal_layer_no(&self, layer: LayerNo) -> i32 {
        let mut found = 0;
        for (i, l) in self.layers.iter().enumerate() {
            if i as LayerNo == layer {
                return found;
            }
            if l.is_signal {
                found += 1;
            }
        }
        -1
    }

    /// Gets the layer number of the `signal_layer_no`-th signal layer.
    pub fn get_layer_no(&self, signal_layer_no: i32) -> LayerNo {
        self.get_signal_layer_index(signal_layer_no) as LayerNo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ls() -> LayerStructure {
        LayerStructure::new(vec![
            Layer::new("F.Cu", true),
            Layer::new("GND", false),
            Layer::new("In2", true),
            Layer::new("B.Cu", true),
        ])
    }

    #[test]
    fn lookups() {
        let s = ls();
        assert_eq!(s.get_no("GND"), 1);
        assert_eq!(s.get_no("gnd"), -1);
        assert_eq!(s.signal_layer_count(), 3);
        assert_eq!(s.get_layer_no(0), 0);
        assert_eq!(s.get_layer_no(1), 2);
        assert_eq!(s.get_layer_no(2), 3);
        assert_eq!(s.get_layer_no(7), 3);
        assert_eq!(s.get_signal_layer_no(3), 2);
        assert_eq!(s.get_signal_layer_no(1), 1);
        assert_eq!(s.get_signal_layer_no(9), -1);
        assert_eq!(s.get_signal_layer(1).name, "In2");
    }
}
