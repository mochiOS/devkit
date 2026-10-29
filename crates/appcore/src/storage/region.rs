//! Explicit native storage region; no device IDs are exposed to applications.
use super::Observation;
use crate::{Error, Result};
#[derive(Debug)]
pub struct StorageRegion {
    bytes: u64,
    #[cfg(target_os = "mochios")]
    handle: u32,
}
impl StorageRegion {
    /// Blocking worker operation. Supply the epoch captured during review,
    /// an explicit delegated endpoint, and a region within the observed disk.
    /// The owner checks the real sender's device.storage.scoped.write grant.
    #[cfg(target_os = "mochios")]
    pub fn acquire(
        endpoint: u32,
        observation: &Observation,
        disk: u64,
        first: u64,
        sectors: u64,
        epoch: u64,
    ) -> Result<Self> {
        use mochi_user_platform::{handle, ipc};
        use mochios_filesystem_protocol::storage_region as wire;
        let bytes = sectors
            .checked_mul(512)
            .filter(|n| *n != 0)
            .ok_or(Error::InvalidArgument)?;
        let target = observation
            .snapshot
            .entries
            .iter()
            .find(|e| e.id == disk && e.kind == super::Kind::Disk)
            .ok_or(Error::InvalidArgument)?;
        if epoch == 0
            || endpoint == 0
            || observation.snapshot.physical_inventory_unavailable
            || target.sector_bytes != 512
            || first
                .checked_add(sectors)
                .and_then(|n| n.checked_mul(512))
                .is_none_or(|n| n > target.capacity)
        {
            return Err(Error::InvalidArgument);
        }
        let request = wire::Request {
            generation: observation.generation,
            disk,
            first,
            sectors,
            epoch,
        };
        let mut response = [0; 8];
        let mut attached = ipc::IpcObjectHandles::default();
        let length =
            ipc::call_object_handles(endpoint, &request.encode(), &mut response, &mut attached)
                .map_err(Error::from)? as u32 as usize;
        let refs = attached
            .handles
            .iter()
            .take(attached.count as usize)
            .map(|h| (h.handle, h.reserved, h.rights))
            .collect::<Vec<_>>();
        let rights = handle::HANDLE_RIGHT_READ
            | handle::HANDLE_RIGHT_WRITE
            | handle::HANDLE_RIGHT_CONTROL
            | handle::HANDLE_RIGHT_TRANSFER;
        let raw = super::reservation::take_reply(
            if length == 8 { &response } else { &[] },
            attached.delivery_flags,
            attached.count as usize,
            &refs,
            rights,
            |h| {
                let _ = handle::close(h);
            },
        )?;
        Ok(Self { bytes, handle: raw })
    }
    #[cfg(not(target_os = "mochios"))]
    pub fn acquire(_: u32, _: &Observation, _: u64, _: u64, _: u64, _: u64) -> Result<Self> {
        Err(Error::UnsupportedPlatform)
    }
    pub fn byte_len(&self) -> u64 {
        self.bytes
    }
    fn check(&self, offset: u64, len: usize) -> Result<()> {
        if offset % 512 != 0
            || len % 512 != 0
            || offset
                .checked_add(len as u64)
                .is_none_or(|n| n > self.bytes)
        {
            Err(Error::InvalidArgument)
        } else {
            Ok(())
        }
    }
    pub fn read_at(&self, offset: u64, bytes: &mut [u8]) -> Result<()> {
        self.check(offset, bytes.len())?;
        #[cfg(target_os = "mochios")]
        {
            for (index, part) in bytes.chunks_mut(4096).enumerate() {
                let count = mochi_user_platform::handle::read_at(
                    self.handle,
                    part,
                    offset + (index * 4096) as u64,
                )
                .map_err(Error::from)?;
                if count != part.len() {
                    return Err(Error::System(5));
                }
            }
            Ok(())
        }
        #[cfg(not(target_os = "mochios"))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }
    pub fn write_at(&self, offset: u64, bytes: &[u8]) -> Result<()> {
        self.check(offset, bytes.len())?;
        #[cfg(target_os = "mochios")]
        {
            for (index, part) in bytes.chunks(4096).enumerate() {
                let count = mochi_user_platform::handle::write_at(
                    self.handle,
                    part,
                    offset + (index * 4096) as u64,
                )
                .map_err(Error::from)?;
                if count != part.len() {
                    return Err(Error::System(5));
                }
            }
            Ok(())
        }
        #[cfg(not(target_os = "mochios"))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }
    pub fn flush(&self) -> Result<()> {
        #[cfg(target_os = "mochios")]
        {
            let length = mochi_user_platform::handle::control(
                self.handle,
                mochios_filesystem_protocol::storage_region::FLUSH,
                &[],
                &mut [],
            )
            .map_err(Error::from)?;
            if length != 0 {
                return Err(Error::System(5));
            }
            Ok(())
        }
        #[cfg(not(target_os = "mochios"))]
        {
            Err(Error::UnsupportedPlatform)
        }
    }
}
#[cfg(target_os = "mochios")]
impl Drop for StorageRegion {
    fn drop(&mut self) {
        let _ = mochi_user_platform::handle::close(self.handle);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_overflow_and_unaligned_regions() {
        let r = StorageRegion { bytes: 1024 };
        assert!(r.check(512, 512).is_ok());
        assert!(r.check(1024, 0).is_ok());
        for (o, l) in [(1, 512), (512, 513), (1024, 512), (u64::MAX, 512)] {
            assert!(r.check(o, l).is_err());
        }
    }
}
