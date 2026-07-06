use super::{Connection, TxnResources};
use kuzu_binder::bound_statement::BoundStatement;
use kuzu_storage::{LocalStorage, LocalWAL, ShadowFile};
use kuzu_transaction::Transaction;

impl Connection {
    /// Begin a write transaction and allocate per-txn resources.
    /// Returns the transaction on success.
    pub(crate) fn begin_write_txn(&self) -> Result<Transaction, String> {
        let tm = &self.database.transaction_manager;
        let txn = tm.begin_write()?;
        let resources = TxnResources {
            local_storage: LocalStorage::new(),
            local_wal: LocalWAL::new(),
            shadow_file: ShadowFile::new(),
        };
        self.txn_resources
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?
            .insert(txn.transaction_id, resources);
        Ok(txn)
    }

    /// Commit a write transaction: flush resources and clean up.
    ///
    /// The commit pipeline delegates to `StorageManager::commit_transaction()`
    /// which handles: WAL append+flush, LocalStorage flush to tables,
    /// ShadowFile apply to BufferManager, and auto-checkpoint.
    pub(crate) fn commit_write_txn(&self, txn: &mut Transaction) -> Result<(), String> {
        let txn_id = txn.transaction_id;
        // Take the resources out of the map
        let resources = self
            .txn_resources
            .lock()
            .map_err(|e| format!("Lock error: {e}"))?
            .remove(&txn_id);
        let resources = match resources {
            Some(r) => r,
            None => return Err(format!("No resources found for txn#{}", txn_id)),
        };

        // Step 1: Commit via TransactionManager (assigns commit_ts, releases locks)
        let tm = &self.database.transaction_manager;
        let commit_result = tm.commit(txn);
        let _commit_ts = match commit_result {
            kuzu_transaction::CommitResult::Committed { commit_ts } => commit_ts,
        };

        // Step 2: Bulk-copy LocalWAL buffer into global WAL (before flush)
        {
            let sm = &self.database.storage_manager;
            let mut wal = sm.wal().lock().map_err(|e| format!("WAL lock: {e}"))?;
            if !resources.local_wal.is_empty() {
                wal.write_raw_buffer(resources.local_wal.buffer());
            }
        }

        // Step 3: Flush LocalStorage → tables, ShadowFile → BM, WAL + checkpoint
        // `commit_transaction` handles: Commit record append, WAL flush to disk,
        // LocalStorage flush to tables, ShadowFile apply to BM, auto-checkpoint.
        let sm = &self.database.storage_manager;
        sm.commit_transaction(
            &resources.local_storage,
            &resources.shadow_file,
            self.database.config.checkpoint_threshold,
            txn_id,
        )?;

        Ok(())
    }

    /// Rollback a write transaction: discard resources.
    pub(crate) fn rollback_write_txn(&self, txn: &mut Transaction) -> Vec<kuzu_transaction::UndoRecord> {
        let txn_id = txn.transaction_id;
        // Remove resources (discard them) — try to get them for cleanup
        let resources = self.txn_resources.lock().ok().and_then(|mut map| map.remove(&txn_id));

        // Rollback via TransactionManager
        let tm = &self.database.transaction_manager;
        let records = tm.rollback(txn);

        // Rollback in StorageManager too (if we have resources)
        if let Some(mut res) = resources {
            let sm = &self.database.storage_manager;
            let _ = sm.rollback_transaction(
                &mut res.local_storage,
                &mut res.shadow_file,
                txn_id,
                &records.to_vec(),
            );
        }

        records
    }

    pub(crate) fn is_write_statement(bound: &BoundStatement) -> bool {
        match bound {
            BoundStatement::BoundCreateNodeTable(_)
            | BoundStatement::BoundCreateRelTable(_)
            | BoundStatement::BoundDropTable(_)
            | BoundStatement::BoundCreateVectorIndex(_)
            | BoundStatement::BoundCreateDml(_)
            | BoundStatement::BoundMerge(_)
            | BoundStatement::BoundCopyFrom(_)
            | BoundStatement::BoundAlterTable(_)
            | BoundStatement::BoundExportDatabase(_)
            | BoundStatement::BoundImportDatabase(_) => true,
            BoundStatement::BoundExplain(_) => false,
            BoundStatement::BoundQuery(q) => q.clauses.iter().any(|c| {
                matches!(
                    c,
                    kuzu_binder::bound_statement::BoundClause::BoundSet(_)
                        | kuzu_binder::bound_statement::BoundClause::BoundDelete(_)
                        | kuzu_binder::bound_statement::BoundClause::BoundCreate(_)
                )
            }),
            _ => false,
        }
    }

    /// Extract all table IDs that will be written to by this statement.
    pub(crate) fn extract_write_tables(bound: &BoundStatement) -> Vec<u64> {
        let mut table_ids = Vec::new();
        match bound {
            BoundStatement::BoundCopyFrom(c) => table_ids.push(c.table_id),
            BoundStatement::BoundQuery(q) => {
                for clause in &q.clauses {
                    match clause {
                        kuzu_binder::bound_statement::BoundClause::BoundSet(s) => {
                            for item in &s.items {
                                table_ids.push(item.table_id);
                            }
                        }
                        kuzu_binder::bound_statement::BoundClause::BoundDelete(d) => {
                            for item in &d.items {
                                table_ids.push(item.table_id);
                            }
                        }
                        kuzu_binder::bound_statement::BoundClause::BoundCreate(c) => {
                            for p in &c.patterns {
                                if let Some(id) = p.node_table_id {
                                    table_ids.push(id);
                                }
                                if let Some(ref e) = p.edge {
                                    if let Some(id) = e.rel_table_id {
                                        table_ids.push(id);
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            BoundStatement::BoundCreateDml(c) => {
                table_ids.push(c.table_id);
            }
            BoundStatement::BoundMerge(m) => {
                table_ids.push(m.table_id);
                for item in &m.on_create {
                    table_ids.push(item.table_id);
                }
                for item in &m.on_match {
                    table_ids.push(item.table_id);
                }
            }
            _ => {}
        }
        table_ids.sort_unstable();
        table_ids.dedup();
        table_ids
    }
}
