//! People persistence: subjects, faces, face-graph edges, merge suggestions.
use anyhow::Result;
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use crate::people::models::{Subject, Face};
