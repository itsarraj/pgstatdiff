//! Pure delta computation between two snapshots — no database, no clock,
//! fully unit-testable. `pg_stat_database`/`pg_stat_user_tables` columns
//! are cumulative counters, so "what happened between snapshot A and
//! snapshot B" is just `B - A`, plus the handful of derived rates that
//! division by the elapsed wall-clock time gives you.

use std::collections::HashMap;

use serde::Serialize;
use thiserror::Error;

use crate::model::Snapshot;

#[derive(Debug, Error, PartialEq)]
pub enum DiffError {
    #[error(
        "snapshots are from different databases ({before:?} vs {after:?}) — diffing across databases isn't meaningful"
    )]
    DifferentDatabase { before: String, after: String },
    #[error("the \"after\" snapshot ({after}) is not later than the \"before\" snapshot ({before}) — pass them in chronological order")]
    NonPositiveElapsed {
        before: chrono::DateTime<chrono::Utc>,
        after: chrono::DateTime<chrono::Utc>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TableDelta {
    pub schema: String,
    pub table: String,
    pub seq_scan: i64,
    pub idx_scan: i64,
    pub n_tup_ins: i64,
    pub n_tup_upd: i64,
    pub n_tup_del: i64,
    pub live_tup_delta: i64,
    pub dead_tup_delta: i64,
}

impl TableDelta {
    pub fn total_writes(&self) -> i64 {
        self.n_tup_ins + self.n_tup_upd + self.n_tup_del
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DeltaReport {
    pub elapsed_secs: f64,
    pub reset_detected: bool,

    pub commits: i64,
    pub rollbacks: i64,
    pub commits_per_sec: f64,
    pub rollbacks_per_sec: f64,

    /// Cache hit ratio computed over just this interval's reads
    /// (`delta blks_hit / (delta blks_hit + delta blks_read)`), not the
    /// server's lifetime ratio — a server that's been up for a month
    /// with a 99.9% lifetime hit ratio can still have a terrible last 30
    /// seconds, and the lifetime number would never show it.
    pub cache_hit_ratio: Option<f64>,

    pub rows_read: i64,
    pub rows_written: i64,
    pub rows_read_per_sec: f64,
    pub rows_written_per_sec: f64,

    pub temp_files: i64,
    pub temp_bytes: i64,
    pub deadlocks: i64,

    pub numbackends_before: i64,
    pub numbackends_after: i64,

    /// Per-table deltas, sorted by total rows written (ins+upd+del)
    /// descending — the tables that were actually busy during the
    /// interval float to the top.
    pub tables: Vec<TableDelta>,
}

fn non_negative_delta(before: i64, after: i64, reset_detected: &mut bool) -> i64 {
    let d = after - before;
    if d < 0 {
        *reset_detected = true;
        // Counters were reset mid-interval (pg_stat_reset(), or a server
        // restart clearing pg_stat_database). The only honest thing left
        // to attribute to "since the reset" is the post-reset value
        // itself, so that's what gets reported instead of a nonsense
        // negative number.
        after
    } else {
        d
    }
}

pub fn compute(before: &Snapshot, after: &Snapshot) -> Result<DeltaReport, DiffError> {
    if before.database_name != after.database_name {
        return Err(DiffError::DifferentDatabase {
            before: before.database_name.clone(),
            after: after.database_name.clone(),
        });
    }
    let elapsed = (after.taken_at - before.taken_at).num_milliseconds() as f64 / 1000.0;
    if elapsed <= 0.0 {
        return Err(DiffError::NonPositiveElapsed {
            before: before.taken_at,
            after: after.taken_at,
        });
    }

    let mut reset_detected = before.database.stats_reset != after.database.stats_reset
        && after.database.stats_reset.is_some();

    let b = &before.database;
    let a = &after.database;

    let commits = non_negative_delta(b.xact_commit, a.xact_commit, &mut reset_detected);
    let rollbacks = non_negative_delta(b.xact_rollback, a.xact_rollback, &mut reset_detected);
    let blks_read = non_negative_delta(b.blks_read, a.blks_read, &mut reset_detected);
    let blks_hit = non_negative_delta(b.blks_hit, a.blks_hit, &mut reset_detected);
    let tup_returned = non_negative_delta(b.tup_returned, a.tup_returned, &mut reset_detected);
    let tup_fetched = non_negative_delta(b.tup_fetched, a.tup_fetched, &mut reset_detected);
    let tup_inserted = non_negative_delta(b.tup_inserted, a.tup_inserted, &mut reset_detected);
    let tup_updated = non_negative_delta(b.tup_updated, a.tup_updated, &mut reset_detected);
    let tup_deleted = non_negative_delta(b.tup_deleted, a.tup_deleted, &mut reset_detected);
    let temp_files = non_negative_delta(b.temp_files, a.temp_files, &mut reset_detected);
    let temp_bytes = non_negative_delta(b.temp_bytes, a.temp_bytes, &mut reset_detected);
    let deadlocks = non_negative_delta(b.deadlocks, a.deadlocks, &mut reset_detected);

    let cache_hit_ratio = if blks_hit + blks_read > 0 {
        Some(blks_hit as f64 / (blks_hit + blks_read) as f64)
    } else {
        None
    };

    let rows_read = tup_returned + tup_fetched;
    let rows_written = tup_inserted + tup_updated + tup_deleted;

    let before_tables: HashMap<(&str, &str), &crate::model::TableStat> =
        before.tables.iter().map(|t| (t.key(), t)).collect();

    let mut table_deltas = Vec::new();
    for t_after in &after.tables {
        let key = t_after.key();
        if let Some(t_before) = before_tables.get(&key) {
            table_deltas.push(TableDelta {
                schema: t_after.schema.clone(),
                table: t_after.table.clone(),
                seq_scan: non_negative_delta(
                    t_before.seq_scan,
                    t_after.seq_scan,
                    &mut reset_detected,
                ),
                idx_scan: non_negative_delta(
                    t_before.idx_scan,
                    t_after.idx_scan,
                    &mut reset_detected,
                ),
                n_tup_ins: non_negative_delta(
                    t_before.n_tup_ins,
                    t_after.n_tup_ins,
                    &mut reset_detected,
                ),
                n_tup_upd: non_negative_delta(
                    t_before.n_tup_upd,
                    t_after.n_tup_upd,
                    &mut reset_detected,
                ),
                n_tup_del: non_negative_delta(
                    t_before.n_tup_del,
                    t_after.n_tup_del,
                    &mut reset_detected,
                ),
                live_tup_delta: t_after.n_live_tup - t_before.n_live_tup,
                dead_tup_delta: t_after.n_dead_tup - t_before.n_dead_tup,
            });
        }
        // A table present only in `after` (created mid-interval) has no
        // "before" row to diff against and is skipped rather than
        // reported with a fabricated baseline of zero — a brand-new
        // table's absolute counts aren't a "delta" in any meaningful
        // sense.
    }
    table_deltas.sort_by_key(|t| std::cmp::Reverse(t.total_writes()));

    Ok(DeltaReport {
        elapsed_secs: elapsed,
        reset_detected,
        commits,
        rollbacks,
        commits_per_sec: commits as f64 / elapsed,
        rollbacks_per_sec: rollbacks as f64 / elapsed,
        cache_hit_ratio,
        rows_read,
        rows_written,
        rows_read_per_sec: rows_read as f64 / elapsed,
        rows_written_per_sec: rows_written as f64 / elapsed,
        temp_files,
        temp_bytes,
        deadlocks,
        numbackends_before: b.numbackends,
        numbackends_after: a.numbackends,
        tables: table_deltas,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DatabaseStat, TableStat};
    use chrono::Duration;

    fn dbstat(commit: i64, blks_read: i64, blks_hit: i64) -> DatabaseStat {
        DatabaseStat {
            numbackends: 3,
            xact_commit: commit,
            xact_rollback: 0,
            blks_read,
            blks_hit,
            tup_returned: 0,
            tup_fetched: 0,
            tup_inserted: 0,
            tup_updated: 0,
            tup_deleted: 0,
            temp_files: 0,
            temp_bytes: 0,
            deadlocks: 0,
            stats_reset: Some(chrono::DateTime::UNIX_EPOCH),
        }
    }

    fn snap(
        at: chrono::DateTime<chrono::Utc>,
        db: DatabaseStat,
        tables: Vec<TableStat>,
    ) -> Snapshot {
        Snapshot {
            taken_at: at,
            database_name: "testdb".into(),
            database: db,
            tables,
        }
    }

    #[test]
    fn basic_delta_and_rates() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(10);
        let before = snap(t0, dbstat(100, 50, 950), vec![]);
        let after = snap(t1, dbstat(150, 60, 1000), vec![]);
        let report = compute(&before, &after).unwrap();
        assert_eq!(report.commits, 50);
        assert!((report.commits_per_sec - 5.0).abs() < 1e-9);
        assert!(!report.reset_detected);
    }

    #[test]
    fn cache_hit_ratio_is_computed_over_the_interval_not_lifetime() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(1);
        // Lifetime hit ratio implied by absolute numbers would be huge
        // (999900/1000000), but *during this interval* every single
        // block read was a miss — 0% for the window that matters.
        let before = dbstat(0, 999_900, 100);
        let after = dbstat(0, 1_000_000, 100);
        let report = compute(&snap(t0, before, vec![]), &snap(t1, after, vec![])).unwrap();
        assert_eq!(report.cache_hit_ratio, Some(0.0));
    }

    #[test]
    fn cache_hit_ratio_none_when_no_block_activity() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(1);
        let before = dbstat(0, 10, 10);
        let after = dbstat(0, 10, 10);
        let report = compute(&snap(t0, before, vec![]), &snap(t1, after, vec![])).unwrap();
        assert_eq!(report.cache_hit_ratio, None);
    }

    #[test]
    fn different_database_names_rejected() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(1);
        let mut before = snap(t0, dbstat(0, 0, 0), vec![]);
        before.database_name = "dbA".into();
        let mut after = snap(t1, dbstat(0, 0, 0), vec![]);
        after.database_name = "dbB".into();
        let err = compute(&before, &after).unwrap_err();
        assert!(matches!(err, DiffError::DifferentDatabase { .. }));
    }

    #[test]
    fn non_positive_elapsed_rejected_when_after_is_earlier() {
        let t0 = chrono::Utc::now();
        let t1 = t0 - Duration::seconds(5);
        let before = snap(t0, dbstat(0, 0, 0), vec![]);
        let after = snap(t1, dbstat(0, 0, 0), vec![]);
        let err = compute(&before, &after).unwrap_err();
        assert!(matches!(err, DiffError::NonPositiveElapsed { .. }));
    }

    #[test]
    fn non_positive_elapsed_rejected_when_equal() {
        let t0 = chrono::Utc::now();
        let before = snap(t0, dbstat(0, 0, 0), vec![]);
        let after = snap(t0, dbstat(5, 0, 0), vec![]);
        let err = compute(&before, &after).unwrap_err();
        assert!(matches!(err, DiffError::NonPositiveElapsed { .. }));
    }

    #[test]
    fn counter_reset_detected_and_uses_post_reset_value() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(10);
        let mut before = dbstat(500, 0, 0);
        before.stats_reset = Some(chrono::DateTime::UNIX_EPOCH);
        let mut after = dbstat(20, 0, 0); // dropped: a reset happened
        after.stats_reset = Some(chrono::DateTime::UNIX_EPOCH + Duration::seconds(5));
        let report = compute(&snap(t0, before, vec![]), &snap(t1, after, vec![])).unwrap();
        assert!(report.reset_detected);
        assert_eq!(report.commits, 20); // the post-reset value, not -480
    }

    fn tstat(schema: &str, table: &str, ins: i64, upd: i64, del: i64) -> TableStat {
        TableStat {
            schema: schema.into(),
            table: table.into(),
            seq_scan: 0,
            idx_scan: 0,
            n_tup_ins: ins,
            n_tup_upd: upd,
            n_tup_del: del,
            n_live_tup: ins - del,
            n_dead_tup: upd + del,
        }
    }

    #[test]
    fn table_deltas_computed_and_sorted_by_total_writes() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(10);
        let before_tables = vec![
            tstat("public", "quiet", 10, 0, 0),
            tstat("public", "busy", 10, 0, 0),
        ];
        let after_tables = vec![
            tstat("public", "quiet", 11, 0, 0),   // +1 write
            tstat("public", "busy", 10, 500, 50), // +550 writes
        ];
        let before = snap(t0, dbstat(0, 0, 0), before_tables);
        let after = snap(t1, dbstat(0, 0, 0), after_tables);
        let report = compute(&before, &after).unwrap();
        assert_eq!(report.tables.len(), 2);
        assert_eq!(report.tables[0].table, "busy");
        assert_eq!(report.tables[0].total_writes(), 550);
        assert_eq!(report.tables[1].table, "quiet");
    }

    #[test]
    fn table_created_mid_interval_is_skipped_not_fabricated() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(10);
        let before = snap(t0, dbstat(0, 0, 0), vec![]);
        let after = snap(
            t1,
            dbstat(0, 0, 0),
            vec![tstat("public", "new_table", 100, 0, 0)],
        );
        let report = compute(&before, &after).unwrap();
        assert!(report.tables.is_empty());
    }

    #[test]
    fn table_dropped_mid_interval_is_absent_from_report() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(10);
        let before = snap(t0, dbstat(0, 0, 0), vec![tstat("public", "gone", 10, 0, 0)]);
        let after = snap(t1, dbstat(0, 0, 0), vec![]);
        let report = compute(&before, &after).unwrap();
        assert!(report.tables.is_empty());
    }

    #[test]
    fn rows_read_and_written_combine_the_right_columns() {
        let t0 = chrono::Utc::now();
        let t1 = t0 + Duration::seconds(1);
        let mut before = dbstat(0, 0, 0);
        before.tup_returned = 0;
        before.tup_fetched = 0;
        before.tup_inserted = 0;
        before.tup_updated = 0;
        before.tup_deleted = 0;
        let mut after = dbstat(0, 0, 0);
        after.tup_returned = 100;
        after.tup_fetched = 50;
        after.tup_inserted = 5;
        after.tup_updated = 3;
        after.tup_deleted = 2;
        let report = compute(&snap(t0, before, vec![]), &snap(t1, after, vec![])).unwrap();
        assert_eq!(report.rows_read, 150);
        assert_eq!(report.rows_written, 10);
    }
}
