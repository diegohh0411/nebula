//! Persistence foundation: pool, init, schema, sqlite-vec registration.
//! Domain queries are being migrated OUT of `legacy` into per-slice `repo.rs`
//! modules (TT-63). `legacy` shrinks to empty and is deleted in the final task.
mod legacy;
pub use legacy::*;
