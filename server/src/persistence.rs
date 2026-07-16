//! SQLite persistence for production/uptime stats history. Wired into
//! `main.rs`'s tick loop, which calls into this module from inside
//! `tokio::task::spawn_blocking` so the synchronous rusqlite calls don't
//! block the async runtime.

use rusqlite::{params, Connection, Result};
use serde::Serialize;

pub fn open_db(path: &str) -> Result<Connection> {
    let conn = Connection::open(path)?;
    init_schema(&conn)?;
    Ok(conn)
}

pub fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS stats_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tick INTEGER NOT NULL,
            robot_count INTEGER NOT NULL,
            conveyor_running INTEGER NOT NULL,
            total_production REAL NOT NULL
        )",
        [],
    )?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS robot_failure_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            tick INTEGER NOT NULL,
            robot_id INTEGER NOT NULL,
            event_type TEXT NOT NULL
        )",
        [],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct StatsRow {
    pub tick: u64,
    pub robot_count: usize,
    pub conveyor_running: bool,
    pub total_production: f32,
}

pub fn insert_stats(conn: &Connection, row: &StatsRow) -> Result<()> {
    conn.execute(
        "INSERT INTO stats_history (tick, robot_count, conveyor_running, total_production) VALUES (?1, ?2, ?3, ?4)",
        params![
            row.tick as i64,
            row.robot_count as i64,
            row.conveyor_running as i64,
            row.total_production as f64
        ],
    )?;
    Ok(())
}

pub fn recent_stats(conn: &Connection, limit: usize) -> Result<Vec<StatsRow>> {
    let mut stmt = conn.prepare(
        "SELECT tick, robot_count, conveyor_running, total_production FROM stats_history ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(StatsRow {
            tick: row.get::<_, i64>(0)? as u64,
            robot_count: row.get::<_, i64>(1)? as usize,
            conveyor_running: row.get::<_, i64>(2)? != 0,
            total_production: row.get::<_, f64>(3)? as f32,
        })
    })?;
    rows.collect()
}

// Deliberately not wired into `main.rs` yet — Task 7 calls into these from
// the tick loop's status-transition detection. Same write-then-wire-later
// pattern as `stats_history` above (see this file's history for precedent);
// `#[allow(dead_code)]` is scoped to just these new items rather than the
// whole module since `insert_stats`/`recent_stats` are already wired.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FailureEvent {
    pub tick: u64,
    pub robot_id: u32,
    pub event_type: String,
}

#[allow(dead_code)]
pub fn insert_failure_event(conn: &Connection, tick: u64, robot_id: u32, event_type: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO robot_failure_events (tick, robot_id, event_type) VALUES (?1, ?2, ?3)",
        params![tick as i64, robot_id as i64, event_type],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn recent_failure_events(conn: &Connection, limit: usize) -> Result<Vec<FailureEvent>> {
    let mut stmt = conn.prepare(
        "SELECT tick, robot_id, event_type FROM robot_failure_events ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |row| {
        Ok(FailureEvent {
            tick: row.get::<_, i64>(0)? as u64,
            robot_id: row.get::<_, i64>(1)? as u32,
            event_type: row.get(2)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_and_read_back_a_stats_row() {
        let conn = test_db();
        let row = StatsRow { tick: 42, robot_count: 3, conveyor_running: true, total_production: 1.5 };
        insert_stats(&conn, &row).unwrap();

        let rows = recent_stats(&conn, 10).unwrap();
        assert_eq!(rows, vec![row]);
    }

    #[test]
    fn recent_stats_returns_newest_first_and_respects_limit() {
        let conn = test_db();
        for tick in 0..5u64 {
            insert_stats(
                &conn,
                &StatsRow { tick, robot_count: 1, conveyor_running: true, total_production: 0.0 },
            )
            .unwrap();
        }

        let rows = recent_stats(&conn, 2).unwrap();
        assert_eq!(rows.iter().map(|r| r.tick).collect::<Vec<_>>(), vec![4, 3]);
    }

    #[test]
    fn recent_stats_on_empty_db_returns_empty_vec() {
        let conn = test_db();
        let rows = recent_stats(&conn, 10).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn insert_and_read_back_a_failure_event() {
        let conn = test_db();
        insert_failure_event(&conn, 42, 3, "failed").unwrap();

        let rows = recent_failure_events(&conn, 10).unwrap();

        assert_eq!(rows, vec![FailureEvent { tick: 42, robot_id: 3, event_type: "failed".to_string() }]);
    }

    #[test]
    fn recent_failure_events_returns_newest_first_and_respects_limit() {
        let conn = test_db();
        for tick in 0..5u64 {
            insert_failure_event(&conn, tick, 1, "failed").unwrap();
        }

        let rows = recent_failure_events(&conn, 2).unwrap();

        assert_eq!(rows.iter().map(|r| r.tick).collect::<Vec<_>>(), vec![4, 3]);
    }

    #[test]
    fn recent_failure_events_on_empty_db_returns_empty_vec() {
        let conn = test_db();
        let rows = recent_failure_events(&conn, 10).unwrap();
        assert!(rows.is_empty());
    }
}
