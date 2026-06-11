mod app;
mod db;
mod platform;
mod library;
mod media;
mod search;
mod people;
mod tags;
mod vision;
mod settings;
pub mod models;
pub mod pipeline;
// legacy flat modules still present, removed as their slices absorb them:
mod clustering;
mod face_quality;
mod face_store;
mod commands;
mod embedder;
mod preprocess;
mod preview;
mod indexer;
mod vector_index;
mod watcher;
mod thumbnail;
pub mod vision_engine;

pub use app::{run, AppState};
