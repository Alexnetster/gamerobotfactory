//! Deterministic simulation core for the robot-arm conveyor factory game.
//!
//! Pure, network-free building blocks: grid/pathfinding, a deterministic
//! parallel tick loop, procedural gait, two-bone arm IK, and a
//! sorted-by-id production aggregator. This crate has no I/O and no
//! server — wiring these pieces into a running server loop, a
//! WebSocket protocol, and a renderer is out of scope here and happens
//! in later plans. `gait` and `production` in particular are not yet
//! called from `sim::tick` — they're library surface for that future
//! wiring, not dead code.

pub mod gait;
pub mod grid;
pub mod ik;
pub mod pathfind;
pub mod posture;
pub mod production;
pub mod sim;
