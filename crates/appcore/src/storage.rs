//! Storage observations and explicit owner-side reservations.
//! Inventory uses READ authority; reservations separately require device.storage.
use crate::{Error, Result};
mod reservation;
#[cfg(any(target_os = "mochios", test))]
use mochios_filesystem_protocol::storage_inventory as wire;
pub use reservation::StorageReservation;

/// Generation is a selection freshness check, not disk authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub generation: u64,
    pub snapshot: Snapshot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Disk,
    Partition,
    Volume,
    OutsidePartitions,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub parent: Option<u64>,
    pub kind: Kind,
    pub name: String,
    pub capacity: u64,
    /// Filesystem-owner accounting; unavailable for unmounted partitions.
    pub used: Option<u64>,
    /// Unknown access remains None rather than implying write permission.
    pub read_only: Option<bool>,
    pub format: Option<String>,
    pub mount: Option<String>,
    pub start_sector: u64,
    pub sector_bytes: u32,
    /// The disk size comes from GPT metadata rather than hardware inquiry.
    pub capacity_from_partition_map: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    /// Mounted volumes remain useful when physical inventory is unavailable.
    pub physical_inventory_unavailable: bool,
}
#[cfg(any(target_os = "mochios", test))]
fn entry(record: wire::Record<'_>) -> Result<Entry> {
    let kind = match record.kind {
        wire::DISK => Kind::Disk,
        wire::PARTITION => Kind::Partition,
        wire::VOLUME => Kind::Volume,
        wire::OUTSIDE_PARTITIONS => Kind::OutsidePartitions,
        _ => return Err(Error::InvalidArgument),
    };
    Ok(Entry {
        id: record.id,
        parent: (record.parent != 0).then_some(record.parent),
        kind,
        name: record.name.into(),
        capacity: record.capacity,
        used: (record.flags & wire::USAGE_KNOWN != 0).then_some(record.used),
        read_only: (record.flags & wire::ACCESS_KNOWN != 0)
            .then_some(record.flags & wire::READ_ONLY != 0),
        format: (!record.format.is_empty()).then(|| record.format.into()),
        mount: (!record.mount.is_empty()).then(|| record.mount.into()),
        start_sector: record.start,
        sector_bytes: record.sector_bytes,
        capacity_from_partition_map: record.flags & wire::CAPACITY_FROM_PARTITION_MAP != 0,
    })
}
#[cfg(any(target_os = "mochios", test))]
fn collect_observation(
    mut query: impl FnMut(wire::Request) -> Result<[u8; wire::RECORD_LEN]>,
) -> Result<Observation> {
    let mut generation = 0;
    let mut entries = vec![];
    let mut warning = None;
    for index in 0..=wire::MAX_RECORDS {
        let bytes = query(wire::Request {
            generation,
            index: index as u64,
        })?;
        let record = wire::Record::decode(&bytes).ok_or(Error::InvalidArgument)?;
        if generation == 0 {
            generation = record.generation;
            warning = Some(record.warning);
        }
        if generation != record.generation || warning != Some(record.warning) {
            return Err(Error::InvalidArgument);
        }
        if record.kind == 0 {
            validate_tree(&entries)?;
            return Ok(Observation {
                generation,
                snapshot: Snapshot {
                    entries,
                    physical_inventory_unavailable: record.warning != 0,
                },
            });
        }
        if index == wire::MAX_RECORDS {
            return Err(Error::InvalidArgument);
        }
        entries.push(entry(record)?);
    }
    Err(Error::InvalidArgument)
}
#[cfg(any(target_os = "mochios", test))]
fn validate_tree(entries: &[Entry]) -> Result<()> {
    use std::collections::BTreeMap;
    let mut ids = BTreeMap::new();
    for e in entries {
        if ids.insert(e.id, e).is_some() {
            return Err(Error::InvalidArgument);
        }
    }
    for e in entries {
        match (e.kind, e.parent.and_then(|id| ids.get(&id))) {
            (Kind::Disk, None) | (Kind::Volume, None) if e.parent.is_none() => {}
            (Kind::Partition | Kind::OutsidePartitions, Some(parent))
                if parent.kind == Kind::Disk => {}
            (Kind::Volume, Some(parent)) if parent.kind == Kind::Partition => {}
            _ => return Err(Error::InvalidArgument),
        }
    }
    Ok(())
}
/// Performs blocking service I/O; call from a worker, never from painting.
#[cfg(target_os = "mochios")]
pub fn inventory_observation() -> Result<Observation> {
    use mochi_user_platform::handle;
    use mochios_posix_client::{Client, ClientError, OpenAtBase, SyscallEndpointTransport};
    fn error(e: ClientError<mochi_user_platform::syscall::SysError>) -> Error {
        match e {
            ClientError::Transport(e) => Error::from(e),
            ClientError::Remote(s) => Error::System(s.unsigned_abs() as i64),
            _ => Error::InvalidArgument,
        }
    }
    struct Root(u32);
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = handle::close(self.0);
        }
    }
    let client =
        Client::<SyscallEndpointTransport>::from_syscall_launch_context().map_err(error)?;
    let opened = client
        .open_at(
            OpenAtBase::ProcessRoot,
            mochios_posix_protocol::OPEN_READ | mochios_posix_protocol::OPEN_DIRECTORY,
            0,
            "/",
        )
        .map_err(error)?;
    let opened_root = Root(opened.handle);
    if opened.rights & handle::HANDLE_RIGHT_READ == 0 {
        return Err(Error::System(13));
    }
    // A normal POSIX open includes description-control/seek/transfer rights.
    // Retain only READ for this API, and release the broader open immediately.
    let root =
        Root(handle::duplicate(opened_root.0, handle::HANDLE_RIGHT_READ).map_err(Error::from)?);
    drop(opened_root);
    for attempt in 0..3 {
        let result = collect_observation(|request| {
            let mut bytes = [0; wire::RECORD_LEN];
            let count = handle::query(root.0, wire::GET, &request.encode(), &mut bytes)
                .map_err(Error::from)?;
            if count != bytes.len() {
                return Err(Error::InvalidArgument);
            }
            Ok(bytes)
        });
        if result != Err(Error::System(11)) || attempt == 2 {
            return result;
        }
    }
    unreachable!()
}
#[cfg(not(target_os = "mochios"))]
pub fn inventory_observation() -> Result<Observation> {
    Err(Error::UnsupportedPlatform)
}

/// Existing read-only API; callers needing a reservation also retain its generation.
pub fn inventory() -> Result<Snapshot> {
    inventory_observation().map(|observed| observed.snapshot)
}

#[cfg(test)]
fn collect(query: impl FnMut(wire::Request) -> Result<[u8; wire::RECORD_LEN]>) -> Result<Snapshot> {
    collect_observation(query).map(|observed| observed.snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_inventory_preserves_mounted_volume_accounting() {
        let snapshot = collect(|request| {
            Ok(if request.index == 0 {
                wire::Record {
                    kind: wire::VOLUME,
                    id: 99,
                    name: "Data",
                    mount: "/",
                    format: "ext4",
                    capacity: 100,
                    used: 40,
                    flags: wire::USAGE_KNOWN | wire::ACCESS_KNOWN | wire::READ_ONLY,
                    ..wire::Record::end(3, 95)
                }
            } else {
                assert_eq!(request.generation, 3);
                wire::Record::end(3, 95)
            }
            .encode()
            .unwrap())
        })
        .unwrap();
        assert!(snapshot.physical_inventory_unavailable);
        assert_eq!(snapshot.entries[0].used, Some(40));
        assert_eq!(snapshot.entries[0].read_only, Some(true));
    }
    #[test]
    fn rejects_orphans_duplicates_and_mixed_snapshots() {
        let part = wire::Record {
            kind: wire::PARTITION,
            id: 2,
            parent: 1,
            name: "Partition 1",
            ..wire::Record::end(3, 0)
        };
        assert!(
            collect(|request| Ok(if request.index == 0 {
                part
            } else {
                wire::Record::end(3, 0)
            }
            .encode()
            .unwrap()))
            .is_err()
        );
        assert!(
            collect(|request| Ok(if request.index < 2 {
                wire::Record {
                    kind: wire::DISK,
                    id: 2,
                    parent: 0,
                    ..part
                }
            } else {
                wire::Record::end(3, 0)
            }
            .encode()
            .unwrap()))
            .is_err()
        );
        assert!(
            collect(|request| Ok(if request.index == 0 {
                wire::Record {
                    kind: wire::DISK,
                    id: 1,
                    parent: 0,
                    ..part
                }
            } else {
                wire::Record::end(4, 0)
            }
            .encode()
            .unwrap()))
            .is_err()
        );
    }
    #[test]
    fn observations_retain_the_owner_generation_without_granting_authority() {
        let observation = collect_observation(|request| {
            assert_eq!(request.index, 0);
            Ok(wire::Record::end(73, 0).encode().unwrap())
        })
        .unwrap();
        assert_eq!(observation.generation, 73);
        assert!(observation.snapshot.entries.is_empty());
    }
}
