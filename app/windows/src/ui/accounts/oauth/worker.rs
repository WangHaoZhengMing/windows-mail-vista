//! Login workers own copied configuration and channels, never native handles.
use mail_core::{
    Result,
    oauth::{self, OAuthAccount, OAuthConfig},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
};

pub(super) struct LoginWorker {
    cancel: Arc<AtomicBool>,
    results: Receiver<Result<OAuthAccount>>,
    thread: Option<JoinHandle<()>>,
}
impl LoginWorker {
    pub fn start(
        config: OAuthConfig,
        login_hint: String,
        open: impl FnOnce(&str) -> Result<()> + Send + 'static,
    ) -> Result<Self> {
        Self::run(move |cancel| oauth::authorize(&config, &login_hint, cancel, open))
    }
    fn run(
        work: impl FnOnce(&AtomicBool) -> Result<OAuthAccount> + Send + 'static,
    ) -> Result<Self> {
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancel);
        let (sender, results) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("winmail-oauth-sign-in".into())
            .spawn(move || {
                let result = work(&signal);
                // Cancellation always wins a late completion, including a completed exchange.
                if !signal.load(Ordering::Acquire) {
                    let _ = sender.send(result);
                }
            })
            .map_err(|error| format!("Could not start sign-in: {error}"))?;
        Ok(Self {
            cancel,
            results,
            thread: Some(thread),
        })
    }
    pub fn poll(&mut self) -> Option<Result<OAuthAccount>> {
        match self.results.try_recv() {
            Ok(value) => Some(value),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err("Sign-in stopped before completion.".into()))
            }
        }
    }
}
impl Drop for LoginWorker {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        // The loopback wait observes cancellation immediately. A bounded token
        // exchange may still be completing, so never block the native UI on join.
        if let Some(thread) = self.thread.take()
            && thread.is_finished()
        {
            let _ = thread.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        net::{Ipv4Addr, TcpListener},
        time::{Duration, Instant},
    };
    #[test]
    fn closing_ui_cancels_the_listener_worker_without_waiting_or_late_delivery() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (ended, stopped) = mpsc::channel();
        let worker = LoginWorker::run(move |cancel| {
            while !cancel.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(5));
            }
            drop(listener);
            let _ = ended.send(());
            Err("Cancelled".into())
        })
        .unwrap();
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_millis(200));
        stopped.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok());
    }
}
