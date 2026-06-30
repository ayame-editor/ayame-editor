use std::io::Read;

use crate::{Error, Result};

/// Read exactly `buf.len()` bytes; `Ok(false)` if EOF before any byte was read,
/// `Err` on a partial read (a truncated record is corruption).
pub(super) fn read_full<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                if filled == 0 {
                    return Ok(false);
                }
                return Err(Error::Search("truncated spill record".into()));
            }
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(true)
}
