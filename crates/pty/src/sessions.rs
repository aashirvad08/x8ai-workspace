use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use x8ai_core::terminal::{SessionId, TerminalSize};

use crate::command::Program;
use crate::session::{Error, Session, SessionEvents};

/// Every live session, by id. Many sessions are the normal case.
#[derive(Default)]
pub struct Sessions {
    last_id: AtomicU32,
    sessions: Mutex<HashMap<SessionId, Arc<Session>>>,
}

impl Sessions {
    pub fn spawn(
        &self,
        program: &Program,
        size: TerminalSize,
        events: Arc<dyn SessionEvents>,
    ) -> Result<Arc<Session>, Error> {
        let id = SessionId(self.last_id.fetch_add(1, Ordering::Relaxed) + 1);
        let session = Session::spawn(id, program, size, events)?;
        self.lock().insert(id, session.clone());
        Ok(session)
    }

    pub fn get(&self, id: SessionId) -> Result<Arc<Session>, Error> {
        self.lock().get(&id).cloned().ok_or(Error::NotFound(id))
    }

    /// Hangs up the session and forgets it.
    pub fn close(&self, id: SessionId) -> Result<(), Error> {
        let session = self.lock().remove(&id).ok_or(Error::NotFound(id))?;
        session.close();
        Ok(())
    }

    /// Hangs up every session, e.g. when the page that owned them reloads.
    pub fn close_all(&self) {
        for session in self.drain() {
            session.close();
        }
    }

    /// For app exit: hangs up every session, gives the processes up to `grace` in
    /// total to exit, then sends SIGKILL to any that remain.
    pub fn shutdown(&self, grace: Duration) {
        let sessions = self.drain();
        for session in &sessions {
            session.close();
        }
        let deadline = Instant::now() + grace;
        for session in &sessions {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !session.wait_for_exit(remaining) {
                session.kill_now();
            }
        }
    }

    /// Whether any session has a job in the foreground (see
    /// [`Session::has_foreground_job`]), so closing it would end a running program.
    pub fn any_foreground_job(&self) -> bool {
        self.lock()
            .values()
            .any(|session| session.has_foreground_job())
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn drain(&self) -> Vec<Arc<Session>> {
        self.lock().drain().map(|(_, session)| session).collect()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<SessionId, Arc<Session>>> {
        self.sessions.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
