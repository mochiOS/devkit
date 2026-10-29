//! Explicit, device-capability-checked reservation of an observed disk.
#[cfg(target_os = "mochios")]
use super::Kind;
use super::Observation;
use crate::{Error, Result};

/// An exclusive filesystem-owner reservation. It grants no raw read/write API.
/// Dropping the last native reference releases it, including process exit.
#[derive(Debug)]
pub struct StorageReservation {
    disk: u64,
    #[cfg(target_os = "mochios")]
    handle: u32,
}
impl StorageReservation {
    /// Blocking operation: call on a worker. `endpoint` is an explicitly
    /// delegated filesystem SEND Handle, not a global service/object number.
    /// The service checks the actual IPC sender's `device.storage.scoped` capability.
    #[cfg(target_os = "mochios")]
    pub fn acquire(endpoint: u32, observation: &Observation, disk: u64) -> Result<Self> {
        use mochi_user_platform::{handle, ipc};
        use mochios_filesystem_protocol::storage_reservation as wire;
        if endpoint == 0
            || observation.generation == 0
            || observation.snapshot.physical_inventory_unavailable
            || !observation
                .snapshot
                .entries
                .iter()
                .any(|entry| entry.kind == Kind::Disk && entry.id == disk)
        {
            return Err(Error::InvalidArgument);
        }
        let mut attached = ipc::IpcObjectHandles::default();
        let mut response = [0; wire::RESPONSE_LEN];
        let length = ipc::call_object_handles(
            endpoint,
            &wire::Request {
                generation: observation.generation,
                disk,
            }
            .encode(),
            &mut response,
            &mut attached,
        )
        .map_err(Error::from)? as u32 as usize;
        let references = attached
            .handles
            .iter()
            .take(attached.count as usize)
            .map(|object| (object.handle, object.reserved, object.rights))
            .collect::<Vec<_>>();
        let rights = handle::HANDLE_RIGHT_READ
            | handle::HANDLE_RIGHT_CONTROL
            | handle::HANDLE_RIGHT_TRANSFER;
        let raw = take_reply(
            if length == response.len() {
                &response
            } else {
                &[]
            },
            attached.delivery_flags,
            attached.count as usize,
            &references,
            rights,
            |raw| {
                let _ = handle::close(raw);
            },
        )?;
        let reservation = Self { disk, handle: raw };
        // Also reject a non-provider reference or a target that changed during
        // publication. RAII closes the received reference on every error.
        reservation.validate()?;
        Ok(reservation)
    }
    #[cfg(not(target_os = "mochios"))]
    pub fn acquire(_endpoint: u32, _observation: &Observation, _disk: u64) -> Result<Self> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn disk(&self) -> u64 {
        self.disk
    }
    /// Re-inspects the target and mounted volumes. A stale/revoked reservation
    /// cannot become valid again; acquire a new one after reviewing the target.
    #[cfg(target_os = "mochios")]
    pub fn validate(&self) -> Result<()> {
        use mochios_filesystem_protocol::storage_reservation as wire;
        let mut response = [0; 8];
        let length =
            mochi_user_platform::handle::query(self.handle, wire::VALIDATE, &[], &mut response)
                .map_err(Error::from)?;
        if length != response.len() || u64::from_le_bytes(response) != self.disk {
            return Err(Error::InvalidArgument);
        }
        Ok(())
    }
    #[cfg(not(target_os = "mochios"))]
    pub fn validate(&self) -> Result<()> {
        Err(Error::UnsupportedPlatform)
    }
}
#[cfg(target_os = "mochios")]
impl Drop for StorageReservation {
    fn drop(&mut self) {
        let _ = mochi_user_platform::handle::close(self.handle);
    }
}

#[cfg(any(target_os = "mochios", test))]
fn take_reply(
    bytes: &[u8],
    delivery: u32,
    count: usize,
    references: &[(u32, u32, u64)],
    rights: u64,
    mut close: impl FnMut(u32),
) -> Result<u32> {
    let result = (|| {
        let status = i64::from_le_bytes(bytes.try_into().map_err(|_| Error::InvalidArgument)?);
        if delivery != 0 || count != references.len() {
            return Err(Error::InvalidArgument);
        }
        if status != 0 {
            return Err(if count == 0 && (-4095..0).contains(&status) {
                Error::System(-status)
            } else {
                Error::InvalidArgument
            });
        }
        if count != 1 {
            return Err(Error::InvalidArgument);
        }
        let (raw, reserved, granted) = references[0];
        if raw == 0 || reserved != 0 || granted != rights {
            return Err(Error::InvalidArgument);
        }
        Ok(raw)
    })();
    if result.is_err() {
        for (index, (raw, _, _)) in references.iter().enumerate() {
            if *raw != 0
                && !references[..index]
                    .iter()
                    .any(|(earlier, _, _)| earlier == raw)
            {
                close(*raw);
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_an_exact_successful_native_reference_is_retained() {
        assert_eq!(
            take_reply(&0i64.to_le_bytes(), 0, 1, &[(8, 0, 37)], 37, |_| panic!(
                "owned result"
            )),
            Ok(8)
        );
        assert_eq!(
            take_reply(&(-13i64).to_le_bytes(), 0, 0, &[], 37, |_| panic!()),
            Err(Error::System(13))
        );
        for (status, delivery, count, references) in [
            (0, 0, 1, vec![(8, 0, 63)]),
            (0, 0, 1, vec![(8, 1, 37)]),
            (0, 1, 1, vec![(8, 0, 37)]),
            (-13, 0, 1, vec![(8, 0, 37)]),
            (0, 0, 2, vec![(8, 0, 37), (8, 0, 37)]),
        ] {
            let mut closed = Vec::new();
            assert!(
                take_reply(
                    &i64::to_le_bytes(status),
                    delivery,
                    count,
                    &references,
                    37,
                    |raw| closed.push(raw)
                )
                .is_err()
            );
            assert_eq!(closed, [8]);
        }
    }
    #[test]
    fn malformed_status_and_surplus_references_are_reclaimed() {
        let mut closed = vec![];
        assert!(
            take_reply(&[0; 7], 0, 2, &[(8, 0, 37), (9, 0, 37)], 37, |raw| closed
                .push(raw))
            .is_err()
        );
        assert_eq!(closed, [8, 9]);
    }
}
