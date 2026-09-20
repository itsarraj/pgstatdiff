# pgstatdiff

Snapshots Postgres's own `pg_stat_database` and `pg_stat_user_tables`
twice across an interval and reports the delta — queries/sec, cache hit
ratio, rows read/written, busiest tables — a poor-man's activity profiler
needing no extension install (`pg_stat_statements` needs
`shared_preload_libraries` and a server restart; this needs nothing
beyond the stats views every Postgres already has on by default).

## Usage

```bash
# manual two-step: snapshot, do something, snapshot again, diff
pgstatdiff snapshot "host=localhost user=app dbname=app" -o before.json
# ... run your migration / load test / suspicious deploy ...
pgstatdiff snapshot "host=localhost user=app dbname=app" -o after.json
pgstatdiff diff before.json after.json

# or one command: snapshot, sleep N seconds, snapshot, diff
pgstatdiff watch "host=localhost user=app dbname=app" --interval 10
pgstatdiff watch "host=localhost user=app dbname=app" --interval 10 --format json
```

## What it reports

Per-database: commits/rollbacks per second, cache hit ratio for the
interval (not the lifetime average `pg_stat_database` itself reports —
this computes it from the *delta* between the two snapshots, so a
recently-restarted server's historically-low lifetime ratio doesn't mask
a real cache problem happening right now), rows read/written per second.
Per-table: the busiest tables by rows written across the interval
(inserts/updates/deletes, sequential vs. index scan counts, live-tuple
delta) — enough to answer "what actually got hit during that deploy"
without `pg_stat_statements`.

## Status: built and verified, with a real interval diff against a real Postgres

- **18 unit tests** (`cargo test --lib`) across `diff` (per-second rate
  math over a real elapsed interval, cache-hit-ratio computed from the
  delta rather than the lifetime counters, a table with no activity in
  the interval correctly excluded from "busiest tables") and `render`
  (both the text and JSON output shapes).
- **Live-verified against a real local Postgres, not just serialized
  fixtures**: created a real scratch database and table, took a real
  `pgstatdiff snapshot`, ran a real 500-row `INSERT` plus a couple of
  real `SELECT`s against it, took a second real snapshot, and ran
  `pgstatdiff diff` on the two real JSON files — the report correctly
  showed 500 rows written attributed to the right table, real commit and
  row-read/write rates for the actual interval, and a 100% cache hit
  ratio (correct, since this was all served from a database small enough
  to be entirely warm in Postgres's own buffer cache).

**Not done / deliberately deferred**: index-level statistics
(`pg_stat_user_indexes` — covered by this workspace's separate `pgindex`
tool instead, which focuses specifically on bloat/unused-index
detection rather than interval deltas); multi-database snapshots in one
run (each `snapshot` call is scoped to `current_database()` — point it
at a different `conninfo` per database); and long-running `watch` with
more than two snapshots (`watch` always takes exactly a before/after
pair, not a continuous rolling profile).
