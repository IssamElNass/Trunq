//! Short-lived async catalog connection shared by database inspectors.

use crate::config::Profile;
use crate::db::result::ResultSet;
use crate::db::session::{Event, Promise, Session};

#[derive(Default)]
pub struct Snapshot {
    session: Option<Session>,
    pending: Option<Promise<Result<Vec<ResultSet>, String>>>,
    started: bool,
    pub error: Option<String>,
}

impl Snapshot {
    #[cfg(test)]
    pub fn ready_fixture() -> Self {
        Self { started: true, ..Self::default() }
    }

    pub fn refresh(&mut self) {
        *self = Self::default();
    }

    pub fn loading(&self) -> bool {
        !self.started || self.pending.is_some()
    }

    pub fn poll(&mut self, profile: Option<&Profile>, db: &str, password: Option<String>, ctx: &egui::Context, sql: &str) -> Option<Vec<ResultSet>> {
        if !self.started {
            self.started = true;
            if let Some(p) = profile {
                let s = Session::connect(p, db, "Catalog inspector", password, ctx);
                // One consistent read-only snapshot across inventory and memberships.
                self.pending = Some(s.batch(format!("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY;\n{sql}\nCOMMIT;")));
                self.session = Some(s);
            } else {
                self.error = Some("Connection profile is unavailable. Reopen this inspector from a saved connection.".into());
            }
        }
        if let Some(s) = &mut self.session {
            for event in s.poll() {
                if let Event::ConnectFailed { error, .. } | Event::Disconnected(error) = event {
                    self.error = Some(error);
                    self.pending = None;
                }
            }
        }
        if let Some(result) = self.pending.as_ref().and_then(Promise::take) {
            self.pending = None;
            self.session = None;
            match result {
                Ok(sets) => {
                    // Simple protocol also returns BEGIN and COMMIT command results.
                    return Some(sets.into_iter().filter(|s| s.ncols() > 0).collect());
                }
                Err(e) => self.error = Some(e),
            }
        }
        if self.pending.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else {
            self.session = None;
        }
        None
    }
}
