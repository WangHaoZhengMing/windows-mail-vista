//! Cooperative cancellation and durable, per-message download checkpoints.
use super::*;
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Default)]
pub struct SyncControl {
    cancelled: Arc<AtomicBool>,
    sockets: Arc<Mutex<Vec<TcpStream>>>,
    directory: Option<PathBuf>,
}
thread_local! { static CURRENT: RefCell<Option<SyncControl>> = const { RefCell::new(None) }; }
impl SyncControl {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory: Some(directory),
            ..Self::default()
        }
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        if let Ok(sockets) = self.sockets.lock() {
            for socket in sockets.iter() {
                let _ = socket.shutdown(std::net::Shutdown::Both);
            }
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
    pub(super) fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err("Synchronization cancelled".into())
        } else {
            Ok(())
        }
    }
    fn path(&self, key: &str) -> Option<PathBuf> {
        self.directory
            .as_ref()
            .map(|root| root.join(format!("{}.eml", crate::message::hash(key.as_bytes()))))
    }
    pub(super) fn load(&self, key: &str) -> Result<Option<Vec<u8>>> {
        self.check()?;
        let Some(path) = self.path(key) else {
            return Ok(None);
        };
        match std::fs::metadata(&path) {
            Ok(meta) if meta.len() <= crate::MAX_MESSAGE as u64 => {
                std::fs::read(path).map(Some).map_err(|e| e.to_string())
            }
            Ok(_) => Err("Download checkpoint exceeds message size limit".into()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    pub(super) fn save(&self, key: &str, raw: &[u8]) -> Result<()> {
        if let Some(path) = self.path(key) {
            std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
            crate::store::atomic_write(&path, raw)?;
        }
        Ok(())
    }
    /// Remove a checkpoint only after the corresponding catalog commit succeeds.
    pub fn committed(&self, key: &str) {
        if let Some(path) = self.path(key) {
            let _ = std::fs::remove_file(path);
        }
    }
    pub(super) fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        struct Restore(Option<SyncControl>);
        impl Drop for Restore {
            fn drop(&mut self) {
                CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
            }
        }
        let _restore = Restore(CURRENT.with(|slot| slot.replace(Some(self.clone()))));
        operation()
    }
}
pub(super) fn current() -> Option<SyncControl> {
    CURRENT.with(|slot| slot.borrow().clone())
}
pub(super) fn register(socket: &TcpStream) -> Result<()> {
    if let Some(control) = current() {
        let mut sockets = control
            .sockets
            .lock()
            .map_err(|_| "Cancellation state unavailable")?;
        control.check()?;
        sockets.push(socket.try_clone().map_err(|e| e.to_string())?);
    }
    Ok(())
}
