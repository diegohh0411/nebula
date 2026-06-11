mod app;
mod db;
mod platform;
mod library;
mod media;
mod search;
mod people;
mod tags;
pub mod vision;
mod settings;
pub mod models;
pub mod pipeline;
// legacy flat modules still present, removed as their slices absorb them:
mod commands;

pub use app::{run, AppState};
