//! Bases (.base) files. Bases are a core Obsidian plugin that renders a
//! database-style view over notes whose frontmatter matches a query.
//!
//! We ship two: `Drivers.base` (table over `Drivers/`) and `Sources Index.base`
//! (table over `Sources/`). The format is YAML — see
//! <https://obsidian.md/help/bases>.

use anyhow::Result;
use std::path::Path;

pub(crate) fn write_all(root: &Path) -> Result<()> {
    std::fs::write(root.join("Drivers.base"), DRIVERS_BASE)?;
    std::fs::write(root.join("Sources Index.base"), SOURCES_BASE)?;
    Ok(())
}

// Bases YAML reference: property names are plain (not `note.X`), groupBy
// is an object with `property` + optional `direction`, and `file.inFolder`
// is a valid filter function. Confirmed against working real-world
// examples — using `note.X` in the order/property declarations causes
// Bases to silently match zero rows even though the filter is fine.
const DRIVERS_BASE: &str = r#"filters:
  and:
    - entity_type == "driver"
properties:
  code:
    displayName: Code
  name:
    displayName: Name
  current_state:
    displayName: Current State
  tier:
    displayName: Tier
views:
  - type: table
    name: Drivers
    order:
      - code
      - name
      - current_state
      - tier
    groupBy:
      property: tier
      direction: ASC
"#;

const SOURCES_BASE: &str = r##"filters:
  and:
    - entity_type == "source"
properties:
  num:
    displayName: "#"
  domain:
    displayName: Domain
  url:
    displayName: URL
views:
  - type: table
    name: Sources
    order:
      - num
      - domain
      - url
"##;
