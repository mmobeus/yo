//! This machine's id: a UUID made once and kept in the state directory, so
//! records and events can say where they came from. Not the hostname, which
//! changes and collides.

use std::fs;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::sys::Sys;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct MachineId(pub String);

impl MachineId {
    /// The id in `paths.machine_id()`, made and written there the first time.
    pub fn get(sys: &dyn Sys, paths: &Paths) -> Result<Self> {
        let file = paths.machine_id();
        match fs::read_to_string(&file) {
            Ok(s) if !s.trim().is_empty() => return Ok(MachineId(s.trim().to_string())),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&file, e)),
        }
        let id = sys.new_session_id();
        crate::write_atomic(&file, format!("{id}\n").as_bytes())?;
        Ok(MachineId(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeSys, TempDir};

    #[test]
    fn made_once_then_kept() {
        let t = TempDir::new().unwrap();
        let sys = FakeSys::new(t.path());
        sys.push_ids(["11111111-1111-4111-8111-111111111111", "other"]);
        let paths = Paths::new(&sys);
        let a = MachineId::get(&sys, &paths).unwrap();
        let b = MachineId::get(&sys, &paths).unwrap();
        assert_eq!(a.0, "11111111-1111-4111-8111-111111111111");
        assert_eq!(a, b);
    }
}
