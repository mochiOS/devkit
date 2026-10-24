//! 暗号化されたOSユーザー別保管庫へのアクセス。呼出元の身元はサービスが確認します。
//! Native callers declare keychain.access and process.inspect (trusted endpoint verification).
use mochios_keychain_protocol::{self as wire, Reply, Request, Result};
use std::sync::atomic::{AtomicU64, Ordering};
pub use wire::{Application, Error, Item, ItemId, Secret};
use zeroize::Zeroizing;
static NEXT: AtomicU64 = AtomicU64::new(1);
#[derive(Clone, Copy, Debug)]
pub struct Status {
    pub initialized: bool,
    pub unlocked: bool,
    pub session_epoch: u64,
}
pub trait Transport {
    fn exchange(&self, frame: &[u8]) -> Result<Zeroizing<Vec<u8>>>;
}
pub struct Keychain<T = SystemTransport> {
    transport: T,
}
impl Keychain<SystemTransport> {
    pub fn connect() -> Result<Self> {
        Ok(Self {
            transport: SystemTransport::connect()?,
        })
    }
}
impl<T: Transport> Keychain<T> {
    pub fn with_transport(transport: T) -> Self {
        Self { transport }
    }
    fn request(&self, r: Request) -> Result<Reply> {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let request = r.encode(id)?;
        let response = self.transport.exchange(&request)?;
        Reply::decode(&response, id)
    }
    fn ok(&self, r: Request) -> Result<()> {
        match self.request(r)? {
            Reply::Ok => Ok(()),
            _ => Err(Error::Invalid),
        }
    }
    pub fn status(&self) -> Result<Status> {
        match self.request(Request::Status)? {
            Reply::Status {
                initialized,
                unlocked,
                session_epoch,
            } => Ok(Status {
                initialized,
                unlocked,
                session_epoch,
            }),
            _ => Err(Error::Invalid),
        }
    }
    pub fn put(&self, service: &str, account: &str, label: &str, secret: Secret) -> Result<ItemId> {
        match self.request(Request::Put {
            service: service.into(),
            account: account.into(),
            label: label.into(),
            secret,
        })? {
            Reply::Id(id) => Ok(id),
            _ => Err(Error::Invalid),
        }
    }
    pub fn get(&self, id: ItemId) -> Result<Secret> {
        match self.request(Request::Get { id })? {
            Reply::Secret(s) => Ok(s),
            _ => Err(Error::Invalid),
        }
    }
    pub fn delete(&self, id: ItemId) -> Result<()> {
        self.ok(Request::Delete { id })
    }
    pub fn items(&self) -> Result<Vec<Item>> {
        self.list(false)
    }
    fn list(&self, manager: bool) -> Result<Vec<Item>> {
        let mut out = vec![];
        let mut generation = 0;
        let mut expected_total = None;
        loop {
            match self.request(Request::List {
                manager,
                offset: out.len() as u16,
                generation,
            })? {
                Reply::Items {
                    generation: next,
                    total,
                    items,
                } => {
                    if (generation != 0 && next != generation)
                        || expected_total.is_some_and(|n| n != total)
                        || out.len() + items.len() > total as usize
                        || items.is_empty() && out.len() < total as usize
                        || items
                            .iter()
                            .any(|item| out.iter().any(|previous: &Item| previous.id == item.id))
                    {
                        return Err(Error::Conflict);
                    }
                    generation = next;
                    expected_total = Some(total);
                    out.extend(items);
                    if out.len() == total as usize {
                        return Ok(out);
                    }
                }
                _ => return Err(Error::Invalid),
            }
        }
    }
    pub fn manager(&self) -> Manager<'_, T> {
        Manager {
            keychain: self,
            epoch: None,
        }
    }
}
/// 管理操作の権限はサービスが検証します。この型の作成だけでは権限は得られません。
pub struct Manager<'a, T: Transport> {
    keychain: &'a Keychain<T>,
    epoch: Option<u64>,
}
impl<T: Transport> Manager<'_, T> {
    /// Bind a user's approval before queuing asynchronous work. An OS lock or
    /// session change invalidates it; never silently refresh/retry approval.
    pub fn at_epoch(mut self, epoch: u64) -> Self {
        self.epoch = Some(epoch);
        self
    }
    fn epoch(&self) -> Result<u64> {
        match self.epoch {
            Some(epoch) => Ok(epoch),
            None => Ok(self.keychain.status()?.session_epoch),
        }
    }
    pub fn initialize(&self, password: Secret) -> Result<()> {
        self.initialize_at_epoch(password, self.epoch()?)
    }
    pub fn initialize_at_epoch(&self, password: Secret, epoch: u64) -> Result<()> {
        self.keychain.ok(Request::Initialize { epoch, password })
    }
    pub fn unlock(&self, password: Secret) -> Result<()> {
        self.unlock_at_epoch(password, self.epoch()?)
    }
    pub fn unlock_at_epoch(&self, password: Secret, epoch: u64) -> Result<()> {
        self.keychain.ok(Request::Unlock { epoch, password })
    }
    pub fn lock(&self) -> Result<()> {
        self.keychain.ok(Request::Lock)
    }
    pub fn items(&self) -> Result<Vec<Item>> {
        self.keychain.list(true)
    }
    pub fn applications(&self) -> Result<Vec<Application>> {
        let mut out = Vec::new();
        let mut generation = 0;
        let mut expected_total = None;
        loop {
            match self.keychain.request(Request::Applications {
                offset: out.len() as u16,
                generation,
            })? {
                Reply::Applications {
                    generation: next,
                    total,
                    applications,
                } => {
                    if (generation != 0 && next != generation)
                        || expected_total.is_some_and(|n| n != total)
                        || out.len() + applications.len() > usize::from(total)
                        || applications.is_empty() && out.len() < usize::from(total)
                        || applications.iter().any(|a| out.contains(a))
                    {
                        return Err(Error::Conflict);
                    }
                    generation = next;
                    expected_total = Some(total);
                    out.extend(applications);
                    if out.len() == usize::from(total) {
                        return Ok(out);
                    }
                }
                _ => return Err(Error::Invalid),
            }
        }
    }
    pub fn reveal(&self, id: ItemId, password: Secret) -> Result<Secret> {
        match self.keychain.request(Request::Reveal {
            epoch: self.epoch()?,
            id,
            password,
        })? {
            Reply::Secret(s) => Ok(s),
            _ => Err(Error::Invalid),
        }
    }
    pub fn delete(&self, id: ItemId, password: Secret) -> Result<()> {
        self.keychain.ok(Request::ManageDelete {
            epoch: self.epoch()?,
            id,
            password,
        })
    }
    pub fn grant(&self, id: ItemId, target: Application, password: Secret) -> Result<()> {
        self.keychain.ok(Request::Grant {
            epoch: self.epoch()?,
            id,
            target,
            revoke: false,
            password,
        })
    }
    pub fn revoke(&self, id: ItemId, target: Application, password: Secret) -> Result<()> {
        self.keychain.ok(Request::Grant {
            epoch: self.epoch()?,
            id,
            target,
            revoke: true,
            password,
        })
    }
    pub fn change_password(&self, old: Secret, new: Secret) -> Result<()> {
        self.keychain.ok(Request::ChangePassword {
            epoch: self.epoch()?,
            old,
            new,
        })
    }
}
#[cfg(target_os = "mochios")]
pub struct SystemTransport {
    endpoint: u64,
}
#[cfg(target_os = "mochios")]
impl SystemTransport {
    fn connect() -> Result<Self> {
        let endpoint = mochi_user_platform::process::find_by_name(wire::SERVICE_NAME)
            .map_err(|_| Error::Unavailable)?;
        if endpoint == 0 {
            return Err(Error::Unavailable);
        }
        let t = Self { endpoint };
        t.verify()?;
        Ok(t)
    }
    fn verify(&self) -> Result<()> {
        let c = mochi_user_platform::process::thread_security_context(self.endpoint)
            .map_err(|_| Error::Denied)?;
        let name = c
            .package_id
            .get(..c.package_id_len as usize)
            .ok_or(Error::Denied)?;
        if c.provenance != 1 || name != b"org.mochios.keychain-service" {
            return Err(Error::Denied);
        }
        Ok(())
    }
}
#[cfg(target_os = "mochios")]
impl Transport for SystemTransport {
    fn exchange(&self, frame: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        self.verify()?;
        let mut reply = Zeroizing::new(vec![0u8; wire::MAX_FRAME]);
        let result = mochi_user_platform::ipc::call(self.endpoint, frame, &mut reply)
            .map_err(|_| Error::Unavailable)?;
        let len = (result & 0xffff_ffff) as usize;
        if len > reply.len() {
            return Err(Error::Invalid);
        }
        reply.truncate(len);
        Ok(reply)
    }
}
#[cfg(target_os = "linux")]
pub struct SystemTransport {
    socket: std::path::PathBuf,
}
#[cfg(target_os = "linux")]
impl SystemTransport {
    fn connect() -> Result<Self> {
        let socket = std::env::var_os("MOCHIOS_PREVIEW_KEYCHAIN_SOCKET")
            .ok_or(Error::Unavailable)?
            .into();
        Ok(Self { socket })
    }
}
#[cfg(target_os = "linux")]
impl Transport for SystemTransport {
    fn exchange(&self, frame: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        use std::{
            io::{Read, Write},
            os::unix::net::UnixStream,
            time::Duration,
        };
        let mut s = UnixStream::connect(&self.socket).map_err(|_| Error::Unavailable)?;
        use std::os::fd::AsRawFd;
        let mut credentials = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut size = std::mem::size_of_val(&credentials) as libc::socklen_t;
        // SAFETY: the live buffer is exactly the size supplied to SO_PEERCRED.
        let result = unsafe {
            libc::getsockopt(
                s.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut size,
            )
        };
        // SAFETY: geteuid takes no pointers and has no preconditions.
        if result != 0
            || size as usize != std::mem::size_of_val(&credentials)
            || credentials.uid != unsafe { libc::geteuid() }
        {
            return Err(Error::Denied);
        }

        s.set_read_timeout(Some(Duration::from_secs(30)))
            .map_err(|_| Error::Unavailable)?;
        s.set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| Error::Unavailable)?;
        s.write_all(&(frame.len() as u32).to_le_bytes())
            .and_then(|_| s.write_all(frame))
            .map_err(|_| Error::Unavailable)?;
        let mut prefix = [0; 4];
        s.read_exact(&mut prefix).map_err(|_| Error::Unavailable)?;
        let n = u32::from_le_bytes(prefix) as usize;
        if n > wire::MAX_FRAME {
            return Err(Error::Invalid);
        }
        let mut reply = Zeroizing::new(vec![0; n]);
        s.read_exact(&mut reply).map_err(|_| Error::Unavailable)?;
        Ok(reply)
    }
}
#[cfg(not(any(target_os = "linux", target_os = "mochios")))]
pub struct SystemTransport;
#[cfg(not(any(target_os = "linux", target_os = "mochios")))]
impl SystemTransport {
    fn connect() -> Result<Self> {
        Err(Error::Unavailable)
    }
}
#[cfg(not(any(target_os = "linux", target_os = "mochios")))]
impl Transport for SystemTransport {
    fn exchange(&self, _: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        Err(Error::Unavailable)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    struct Loopback;
    impl Transport for Loopback {
        fn exchange(&self, b: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
            let (id, r) = Request::decode(b)?;
            match r {
                Request::Status => Reply::Status {
                    session_epoch: 1,
                    initialized: true,
                    unlocked: false,
                }
                .encode(id),
                _ => Ok(Reply::error(id, Error::Denied)),
            }
        }
    }
    #[test]
    fn bound_management_never_silently_refreshes_stale_approval() {
        struct Stale;
        impl Transport for Stale {
            fn exchange(&self, bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
                let (id, request) = Request::decode(bytes)?;
                match request {
                    Request::Reveal { epoch: 7, .. } => Ok(Reply::error(id, Error::Conflict)),
                    _ => panic!("approval was refreshed or retried instead of remaining bound"),
                }
            }
        }
        let keychain = Keychain::with_transport(Stale);
        assert!(matches!(
            keychain
                .manager()
                .at_epoch(7)
                .reveal([1; 16], Secret::new(b"test password".to_vec())),
            Err(Error::Conflict)
        ));
    }
    #[test]
    fn management_api_does_not_grant_its_own_authority() {
        let k = Keychain::with_transport(Loopback);
        assert!(!k.status().unwrap().unlocked);
        assert_eq!(
            k.manager().unlock(Secret::new(b"a long password".to_vec())),
            Err(Error::Denied)
        );
    }
}
