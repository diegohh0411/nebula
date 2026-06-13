mod app;
mod db;
mod library;
mod media;
pub mod models;
mod people;
pub mod pipeline;
mod platform;
mod search;
mod settings;
mod tags;
pub mod vision;
// legacy flat modules still present, removed as their slices absorb them:
mod commands;

pub use app::{run, AppState};
