//! Port of `board/state/Communication.java` (with `SpecctraParserInfo`). Board observers are
//! dropped (headless). The id generator is [`crate::datastructures::ItemIdGenerator`].

use super::coordinate_transform::CoordinateTransform;
use super::unit::Unit;
use crate::datastructures::ItemIdGenerator;

/// Resolution metadata for Specctra DSN export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteResolution {
    pub char_name: String,
    pub positive_int: i32,
}

/// Information from the parser scope of a Specctra DSN file; optional fields are `None` for
/// Java `null`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpecctraParserInfo {
    /// Character for quoting strings in a DSN file.
    pub string_quote: String,
    pub host_cad: Option<String>,
    pub host_version: Option<String>,
    pub constants: Option<Vec<Vec<String>>>,
    pub write_resolution: Option<WriteResolution>,
    pub dsn_file_generated_by_host: bool,
}

impl Default for SpecctraParserInfo {
    /// `new SpecctraParserInfo("\"", null, null, null, null, false)`.
    fn default() -> Self {
        SpecctraParserInfo {
            string_quote: "\"".to_string(),
            host_cad: None,
            host_version: None,
            constants: None,
            write_resolution: None,
            dsn_file_generated_by_host: false,
        }
    }
}

/// Communication information to host systems or host design formats.
#[derive(Clone, Debug, PartialEq)]
pub struct Communication {
    /// For coordinate transforms to a Specctra DSN file.
    pub coordinate_transform: CoordinateTransform,
    /// Mil, inch or mm.
    pub unit: Unit,
    /// The resolution (1 / unit factor) of the host coordinate system.
    pub resolution: i32,
    /// Java allows `null`; every Java constructor call passes an instance.
    pub specctra_parser_info: Option<SpecctraParserInfo>,
    pub id_generator: ItemIdGenerator,
}

impl Default for Communication {
    /// Java `Communication()`.
    fn default() -> Self {
        Communication::new(
            Unit::Mil,
            1,
            Some(SpecctraParserInfo::default()),
            CoordinateTransform::new(1.0, 0.0, 0.0),
            ItemIdGenerator::new(),
        )
    }
}

impl Communication {
    pub fn new(
        unit: Unit,
        resolution: i32,
        specctra_parser_info: Option<SpecctraParserInfo>,
        coordinate_transform: CoordinateTransform,
        id_generator: ItemIdGenerator,
    ) -> Self {
        Communication {
            coordinate_transform,
            unit,
            resolution,
            specctra_parser_info,
            id_generator,
        }
    }

    fn host_cad(&self) -> Option<&str> {
        self.specctra_parser_info
            .as_ref()
            .and_then(|i| i.host_cad.as_deref())
    }

    /// Whether the host CAD is Autodesk Fusion or legacy EAGLE.
    pub fn host_cad_is_fusion(&self) -> bool {
        match self.host_cad() {
            None => false,
            Some(cad) => {
                let cad = cad.to_lowercase();
                cad.contains("fusion") || cad.contains("cadsoft") || cad.contains("eagle")
            }
        }
    }

    /// Whether the host CAD is KiCad.
    pub fn host_cad_is_kicad(&self) -> bool {
        self.host_cad()
            .is_some_and(|cad| cad.to_lowercase().contains("kicad"))
    }

    /// Whether the host is KiCad with a version <= 5 (first digit run of the host version).
    pub fn host_is_old_kicad(&self) -> bool {
        let Some(info) = &self.specctra_parser_info else {
            return false;
        };
        let (Some(cad), Some(version)) = (&info.host_cad, &info.host_version) else {
            return false;
        };
        if cad.to_lowercase().contains("kicad") {
            // Java: Pattern "\\d+" (ASCII digits), Integer.parseInt of the first match.
            let bytes = version.as_bytes();
            if let Some(start) = bytes.iter().position(u8::is_ascii_digit) {
                let end = bytes[start..]
                    .iter()
                    .position(|b| !b.is_ascii_digit())
                    .map_or(bytes.len(), |e| start + e);
                // Java throws NumberFormatException on int overflow; treated as "not old".
                return version[start..end].parse::<i32>().is_ok_and(|v| v <= 5);
            }
        }
        false
    }

    /// Whether host CAD information exists.
    pub fn host_cad_exists(&self) -> bool {
        self.host_cad().is_some()
    }

    /// Returns the resolution scaled to the input unit.
    pub fn get_resolution(&self, unit: Unit) -> f64 {
        Unit::scale(self.resolution as f64, unit, self.unit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datastructures::IdGenerator;

    fn with_host(cad: Option<&str>, version: Option<&str>) -> Communication {
        let mut c = Communication::default();
        let info = c.specctra_parser_info.as_mut().unwrap();
        info.host_cad = cad.map(str::to_string);
        info.host_version = version.map(str::to_string);
        c
    }

    #[test]
    fn host_checks() {
        assert!(!Communication::default().host_cad_exists());
        assert!(with_host(Some("KiCad's Pcbnew"), Some("(5.1.9)-1")).host_is_old_kicad());
        assert!(!with_host(Some("KiCad's Pcbnew"), Some("7.0.1")).host_is_old_kicad());
        assert!(!with_host(Some("KiCad"), Some("abc")).host_is_old_kicad());
        assert!(!with_host(Some("KiCad"), None).host_is_old_kicad());
        assert!(with_host(Some("KICAD"), None).host_cad_is_kicad());
        assert!(with_host(Some("CadSoft EAGLE"), None).host_cad_is_fusion());
        assert!(!with_host(Some("Altium"), None).host_cad_is_fusion());
    }

    #[test]
    fn resolution_and_ids() {
        let mut c = Communication {
            resolution: 10000,
            unit: Unit::Mm,
            ..Communication::default()
        };
        assert_eq!(c.get_resolution(Unit::Um), 10000.0 * 1.0 / 1000.0);
        assert_eq!(c.id_generator.new_id(), 1);
        assert_eq!(c.id_generator.new_id(), 2);
        assert_eq!(c.id_generator.max_generated_id(), 2);
    }
}
