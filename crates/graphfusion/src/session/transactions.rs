//! Session-owned transactions and cancellation-safe statement participation.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionStatus {
    Idle,
    Active {
        read_only: bool,
        snapshot_seq: CommitSeq,
    },
    Failed {
        read_only: bool,
        snapshot_seq: CommitSeq,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionAction {
    Started,
    Committed,
    RolledBack,
}

#[derive(Debug)]
pub(super) struct ExplicitTransaction {
    // None means failed. Its private data and lifecycle lease have been discarded.
    tx: Option<StatementTxn>,
    read_only: bool,
    snapshot_seq: CommitSeq,
}

impl Session {
    pub fn transaction_status(&self) -> TransactionStatus {
        match &self.transaction {
            None => TransactionStatus::Idle,
            Some(t) if t.tx.is_some() => TransactionStatus::Active {
                read_only: t.read_only,
                snapshot_seq: t.snapshot_seq,
            },
            Some(t) => TransactionStatus::Failed {
                read_only: t.read_only,
                snapshot_seq: t.snapshot_seq,
            },
        }
    }
    pub(super) fn finish_request<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.fail_transaction();
        }
        result
    }
    fn fail_transaction(&mut self) {
        if let Some(transaction) = &mut self.transaction {
            transaction.tx = None;
        }
    }
    pub(super) fn start_transaction(
        &mut self,
        start: &ast::StartTransactionStatement,
    ) -> Result<StatementResult> {
        if self.transaction.is_some() {
            return Err(Error::TransactionActive);
        }
        if start.isolation_level.is_some() {
            return Err(Error::UnsupportedFeature(
                "GQL isolation characteristics".into(),
            ));
        }
        let read_only = start.access_mode == Some(ast::TransactionAccessMode::ReadOnly);
        let mut tx = StatementTxn::begin(&self.db)?;
        tx.set_read_only(read_only);
        let snapshot_seq = tx.base.commit_seq;
        self.transaction = Some(ExplicitTransaction {
            tx: Some(tx),
            read_only,
            snapshot_seq,
        });
        Ok(control(snapshot_seq, TransactionAction::Started))
    }
    pub(super) fn commit_transaction(&mut self) -> Result<StatementResult> {
        let mut transaction = self.transaction.take().ok_or(Error::NoTransaction)?;
        let Some(tx) = transaction.tx.take() else {
            self.transaction = Some(transaction);
            return Err(Error::TransactionFailed);
        };
        match tx.commit() {
            Ok(seq) => Ok(control(seq, TransactionAction::Committed)),
            Err(error) => {
                self.transaction = Some(transaction);
                Err(error)
            }
        }
    }
    pub(super) fn rollback_transaction(&mut self) -> Result<StatementResult> {
        let transaction = self.transaction.take().ok_or(Error::NoTransaction)?;
        Ok(control(
            transaction.snapshot_seq,
            TransactionAction::RolledBack,
        ))
    }
    pub(super) fn close_session(&mut self) -> Result<StatementResult> {
        let transaction = self.transaction.take();
        let action = transaction.as_ref().map(|_| TransactionAction::RolledBack);
        let seq = match transaction {
            Some(t) => t.snapshot_seq,
            None => {
                self.db
                    .inner
                    .state
                    .lock()
                    .map_err(|_| Error::Poisoned)?
                    .commit_seq
            }
        };
        self.state.parameters.clear();
        self.state.closed = true;
        Ok(StatementResult {
            commit_seq: seq,
            affected_objects: 0,
            transaction_pending: false,
            transaction_action: action,
        })
    }
    pub(super) fn begin_statement(&mut self, writes: bool) -> Result<Work<'_>> {
        if self.state.closed {
            return Err(Error::SessionClosed);
        }
        self.db.check_healthy()?;
        if let Some(t) = &self.transaction {
            if t.tx.is_none() {
                return Err(Error::TransactionFailed);
            }
            if writes && t.read_only {
                return Err(Error::ReadOnlyTransaction);
            }
        }
        let (tx, explicit) = if let Some(mut transaction) = self.transaction.take() {
            (transaction.tx.take().unwrap(), Some(transaction))
        } else {
            (StatementTxn::begin(&self.db)?, None)
        };
        Ok(Work {
            session: self,
            tx: Some(tx),
            explicit,
            finished: false,
        })
    }
}

fn control(seq: CommitSeq, action: TransactionAction) -> StatementResult {
    StatementResult {
        commit_seq: seq,
        affected_objects: 0,
        transaction_pending: action == TransactionAction::Started,
        transaction_action: Some(action),
    }
}

/// Own the transaction while an async statement is in flight. Dropping its future
/// discards all pending changes and leaves an explicit transaction failed, rather
/// than returning a partially changed coordinator to the session.
pub(super) struct Work<'a> {
    pub session: &'a mut Session,
    pub tx: Option<StatementTxn>,
    explicit: Option<ExplicitTransaction>,
    finished: bool,
}
impl Work<'_> {
    pub fn finish(mut self, next: Option<SessionState>) -> Result<StatementResult> {
        self.session.db.check_healthy()?;
        let tx = self.tx.take().unwrap();
        let pending = self.explicit.is_some();
        let commit_seq = if let Some(mut explicit) = self.explicit.take() {
            let seq = explicit.snapshot_seq;
            explicit.tx = Some(tx);
            self.session.transaction = Some(explicit);
            seq
        } else {
            tx.commit()?
        };
        if let Some(next) = next {
            self.session.state = next;
        }
        self.finished = true;
        Ok(StatementResult {
            commit_seq,
            affected_objects: 0,
            transaction_pending: pending,
            transaction_action: None,
        })
    }
}
impl Drop for Work<'_> {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(explicit) = self.explicit.take() {
                self.session.transaction = Some(explicit);
            }
        }
    }
}
