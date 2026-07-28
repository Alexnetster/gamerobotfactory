//! Deterministic simulation core for the robot-arm conveyor factory game.
//!
//! Pure, network-free building blocks: grid/pathfinding, a deterministic
//! parallel tick loop, and procedural gait. This crate has no I/O and no
//! server — wiring these pieces into a running server loop, a
//! WebSocket protocol, and a renderer is out of scope here and happens
//! in later plans. `gait` in particular is not yet called from
//! `sim::tick` — it's library surface for that future wiring, not dead
//! code.

pub mod gait;
pub mod grid;
pub mod ik;
pub mod pathfind;
pub mod posture;
pub mod sim;
