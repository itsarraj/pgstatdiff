use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// One `pg_stat_database` row for the connection's current database.
/// Every field is a Postgres cumulative counter — it only ever goes up
/// (or resets to zero via `pg_stat_reset()`), which is exactly what makes
/// two of them, minus each other, a meaningful "what happened in
/// between" rather than a point-in-time gauge.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DatabaseStat {
    pub numbackends: i64,
    pub xact_commit: i64,
    pub xact_rollback: i64,
    pub blks_read: i64,
    pub blks_hit: i64,
    pub tup_returned: i64,
    pub tup_fetched: i64,
    pub tup_inserted: i64,
    pub tup_updated: i64,
    pub tup_deleted: i64,
    pub temp_files: i64,
    pub temp_bytes: i64,
    pub deadlocks: i64,
    /// When the server last reset these counters (`pg_stat_reset()`, or
    /// server start). Comparing this between two snapshots is how a
    /// counter reset mid-interval gets detected rather than silently
    /// misread as a huge negative delta.
    pub stats_reset: Option<DateTime<Utc>>,
}

/// One `pg_stat_user_tables` row, the per-table half of the same
/// snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TableStat {
    pub schema: String,
    pub table: String,
    pub seq_scan: i64,
    pub idx_scan: i64,
    pub n_tup_ins: i64,
    pub n_tup_upd: i64,
    pub n_tup_del: i64,
    pub n_live_tup: i64,
    pub n_dead_tup: i64,
}

impl TableStat {
    pub fn key(&self) -> (&str, &str) {
        (&self.schema, &self.table)
    }
}

/// A full point-in-time capture: server-wide database stats plus every
/// user table, with the wall-clock moment it was taken. Serializable so
/// `pgstatdiff snapshot` can write one to a file and `pgstatdiff diff`
/// can read two back later, minutes or days apart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub taken_at: DateTime<Utc>,
    pub database_name: String,
    pub database: DatabaseStat,
    pub tables: Vec<TableStat>,
}
