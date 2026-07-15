//! SQLite persistence for production/uptime stats history. Deliberately not
//! wired into `main.rs` yet — later tasks in this plan will call into this
//! module from the tick loop (wrapped in `tokio::task::spawn_blocking` so the
//! synchronous rusqlite calls don't block the async runtime). This module is
//! complete and tested on its own first, matching the same
//! write-then-wire-later pattern used for `session.rs` (see that module's
//! history for the precedent): the dead-code lint is suppressed here rather
//! than forcing premature wiring just to satisfy clippy.
#![allow(dead_code)]

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
}
