//! The only part that touches a server: two read-only catalog queries per
//! snapshot, `pg_stat_database` (one row, this connection's database) and
//! `pg_stat_user_tables` (every user table). Nothing here writes.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use postgres::{Client, NoTls, Row};

use crate::model::{DatabaseStat, Snapshot, TableStat};

const DATABASE_SQL: &str = "
SELECT
    datname::text,
    numbackends,
    xact_commit,
    xact_rollback,
    blks_read,
    blks_hit,
    tup_returned,
    tup_fetched,
    tup_inserted,
    tup_updated,
    tup_deleted,
    temp_files,
    temp_bytes,
    deadlocks,
    stats_reset
FROM pg_stat_database
WHERE datname = current_database()
";

const TABLE_SQL: &str = "
SELECT
    schemaname::text,
    relname::text,
    seq_scan,
    idx_scan,
    n_tup_ins,
    n_tup_upd,
    n_tup_del,
    n_live_tup,
    n_dead_tup
FROM pg_stat_user_tables
ORDER BY schemaname, relname
";

pub fn connect(conninfo: &str) -> Result<Client> {
    Client::connect(conninfo, NoTls).context("could not connect to Postgres")
}

fn row_to_database_stat(row: &Row) -> (String, DatabaseStat) {
    let name: String = row.get(0);
    let stat = DatabaseStat {
        numbackends: row.get::<_, i32>(1) as i64,
        xact_commit: row.get(2),
        xact_rollback: row.get(3),
        blks_read: row.get(4),
        blks_hit: row.get(5),
        tup_returned: row.get(6),
        tup_fetched: row.get(7),
        tup_inserted: row.get(8),
        tup_updated: row.get(9),
        tup_deleted: row.get(10),
        temp_files: row.get(11),
        temp_bytes: row.get(12),
        deadlocks: row.get(13),
        stats_reset: row.get::<_, Option<DateTime<Utc>>>(14),
    };
    (name, stat)
}

fn row_to_table_stat(row: &Row) -> TableStat {
    TableStat {
        schema: row.get(0),
        table: row.get(1),
        seq_scan: row.get(2),
        idx_scan: row.get::<_, Option<i64>>(3).unwrap_or(0),
        n_tup_ins: row.get(4),
        n_tup_upd: row.get(5),
        n_tup_del: row.get(6),
        n_live_tup: row.get(7),
        n_dead_tup: row.get(8),
    }
}

pub fn snapshot(client: &mut Client) -> Result<Snapshot> {
    let db_row = client
        .query_one(DATABASE_SQL, &[])
        .context("failed to query pg_stat_database")?;
    let (database_name, database) = row_to_database_stat(&db_row);

    let table_rows = client
        .query(TABLE_SQL, &[])
        .context("failed to query pg_stat_user_tables")?;
    let tables = table_rows.iter().map(row_to_table_stat).collect();

    Ok(Snapshot {
        taken_at: Utc::now(),
        database_name,
        database,
        tables,
    })
}
