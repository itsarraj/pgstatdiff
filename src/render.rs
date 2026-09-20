use crate::diff::DeltaReport;

fn fmt_ratio(r: Option<f64>) -> String {
    match r {
        Some(v) => format!("{:.1}%", v * 100.0),
        None => "n/a".to_string(),
    }
}

fn fmt_bytes(n: i64) -> String {
    let n = n as f64;
    if n >= 1024.0 * 1024.0 {
        format!("{:.1} MiB", n / (1024.0 * 1024.0))
    } else if n >= 1024.0 {
        format!("{:.1} KiB", n / 1024.0)
    } else {
        format!("{n:.0} B")
    }
}

/// How many of the busiest tables to print in the text renderer. JSON
/// output always includes every table with a delta.
const TOP_TABLES: usize = 15;

pub fn render_text(report: &DeltaReport) -> String {
    let mut out = String::new();

    out.push_str(&format!(
        "interval: {:.1}s   backends: {} -> {}\n",
        report.elapsed_secs, report.numbackends_before, report.numbackends_after
    ));
    if report.reset_detected {
        out.push_str(
            "WARNING: a stats counter reset was detected mid-interval (pg_stat_reset() or a server restart) — deltas for affected counters use the post-reset value directly rather than a negative number\n",
        );
    }

    out.push_str(&format!(
        "commits:    {:>8}  ({:.2}/s)\n",
        report.commits, report.commits_per_sec
    ));
    out.push_str(&format!(
        "rollbacks:  {:>8}  ({:.2}/s)\n",
        report.rollbacks, report.rollbacks_per_sec
    ));
    out.push_str(&format!(
        "cache hit ratio (this interval): {}\n",
        fmt_ratio(report.cache_hit_ratio)
    ));
    out.push_str(&format!(
        "rows read:    {:>10}  ({:.1}/s)\n",
        report.rows_read, report.rows_read_per_sec
    ));
    out.push_str(&format!(
        "rows written: {:>10}  ({:.1}/s)\n",
        report.rows_written, report.rows_written_per_sec
    ));
    if report.temp_files > 0 || report.temp_bytes > 0 {
        out.push_str(&format!(
            "temp files: {}  ({} written)\n",
            report.temp_files,
            fmt_bytes(report.temp_bytes)
        ));
    }
    if report.deadlocks > 0 {
        out.push_str(&format!("deadlocks: {}\n", report.deadlocks));
    }

    if !report.tables.is_empty() {
        out.push_str("\nBUSIEST TABLES (by rows written)\n");
        out.push_str(&format!(
            "  {:<24} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8}\n",
            "table", "ins", "upd", "del", "seq_scan", "idx_scan", "live\u{394}"
        ));
        for t in report.tables.iter().take(TOP_TABLES) {
            out.push_str(&format!(
                "  {:<24} {:>8} {:>8} {:>8} {:>10} {:>8} {:>8}\n",
                format!("{}.{}", t.schema, t.table),
                t.n_tup_ins,
                t.n_tup_upd,
                t.n_tup_del,
                t.seq_scan,
                t.idx_scan,
                t.live_tup_delta
            ));
        }
        if report.tables.len() > TOP_TABLES {
            out.push_str(&format!(
                "  ... and {} more table(s) with activity\n",
                report.tables.len() - TOP_TABLES
            ));
        }
    }

    out
}

pub fn render_json(report: &DeltaReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::TableDelta;

    fn sample_report() -> DeltaReport {
        DeltaReport {
            elapsed_secs: 10.0,
            reset_detected: false,
            commits: 100,
            rollbacks: 2,
            commits_per_sec: 10.0,
            rollbacks_per_sec: 0.2,
            cache_hit_ratio: Some(0.97),
            rows_read: 5000,
            rows_written: 300,
            rows_read_per_sec: 500.0,
            rows_written_per_sec: 30.0,
            temp_files: 0,
            temp_bytes: 0,
            deadlocks: 0,
            numbackends_before: 3,
            numbackends_after: 5,
            tables: vec![TableDelta {
                schema: "public".into(),
                table: "orders".into(),
                seq_scan: 1,
                idx_scan: 40,
                n_tup_ins: 200,
                n_tup_upd: 50,
                n_tup_del: 10,
                live_tup_delta: 190,
                dead_tup_delta: 60,
            }],
        }
    }

    #[test]
    fn text_includes_key_numbers() {
        let text = render_text(&sample_report());
        assert!(text.contains("commits:"));
        assert!(text.contains("97.0%"));
        assert!(text.contains("orders"));
    }

    #[test]
    fn text_omits_temp_and_deadlock_lines_when_zero() {
        let text = render_text(&sample_report());
        assert!(!text.contains("temp files"));
        assert!(!text.contains("deadlocks:"));
    }

    #[test]
    fn text_includes_temp_line_when_nonzero() {
        let mut r = sample_report();
        r.temp_files = 3;
        r.temp_bytes = 2_000_000;
        let text = render_text(&r);
        assert!(text.contains("temp files: 3"));
        assert!(text.contains("MiB"));
    }

    #[test]
    fn text_shows_reset_warning_when_detected() {
        let mut r = sample_report();
        r.reset_detected = true;
        let text = render_text(&r);
        assert!(text.contains("WARNING"));
    }

    #[test]
    fn text_no_ratio_renders_n_a_not_nan() {
        let mut r = sample_report();
        r.cache_hit_ratio = None;
        let text = render_text(&r);
        assert!(text.contains("n/a"));
    }

    #[test]
    fn json_round_trips_table_deltas() {
        let json = render_json(&sample_report()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["tables"][0]["table"], "orders");
        assert_eq!(parsed["commits"], 100);
    }

    #[test]
    fn fmt_bytes_picks_the_right_unit() {
        assert_eq!(fmt_bytes(500), "500 B");
        assert_eq!(fmt_bytes(2048), "2.0 KiB");
        assert_eq!(fmt_bytes(5 * 1024 * 1024), "5.0 MiB");
    }
}
