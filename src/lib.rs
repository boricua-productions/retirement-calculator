// Library entry point — exposes all engine modules so integration tests in
// tests/ can reach the simulation primitives.

pub mod config;
pub mod engine;
pub mod handlers;
pub mod models;
pub mod reporter;
pub mod simulation;
pub mod ui;
