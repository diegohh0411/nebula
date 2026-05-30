# Early Preview Creation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make images appear in the gallery as soon as possible after discovery, rather than waiting for the potentially slow ML inference pipeline (SigLIP/Face detection) to finish.

**Architecture:** Move the thumbnail generation block from the end of Stage 2 (inference) to the end of Stage 1 (decode) in the pipeline. Emit an early `image_updated` event to notify the UI to fetch the thumbnail immediately. Keep the existing `image_updated` event at the end of Stage 2 to signal full analysis completion.

**Tech Stack:** Rust (Tauri backend)

---

### Task 1: Move Thumbnail Generation to Stage 1

**Files:**
- Modify: `src-tauri/src/pipeline/mod.rs`

- [ ] **Step 1: Move thumbnail generation code**

In `src-tauri/src/pipeline/mod.rs`, locate the Stage 1 decode loop where `let mut decoded = Vec::new();` is populated (around line 175). We need to process the successfully decoded images *before* they go into Stage 2.

Change this section:
```rust
        let mut decoded = Vec::new();
        for h in handles {
            match h.await {
                Ok(Ok(x)) => decoded.push(x),
                Ok(Err((sem_entry, sub_entry, err_msg))) => {
                    // ... error handling
                }
                Err(e) => eprintln!("[pipeline] decode task panicked: {e}"),
            }
        }
```
to:
```rust
        let mut decoded = Vec::new();
        for h in handles {
            match h.await {
                Ok(Ok(x)) => {
                    let (image_id, _, _, ref d) = x;
                    // Early Thumbnail Generation (Stage 1)
                    let thumb_path = crate::thumbnail::thumbnail_path_for(&data_dir, image_id);
                    let thumb_path_str = thumb_path.to_string_lossy().to_string();
                    let d_thumb = d.clone();
                    let write_ok = tokio::task::spawn_blocking(move || {
                        crate::thumbnail::write_thumbnail_from_image(d_thumb.full.as_ref(), &thumb_path)
                    })
                    .await
                    .map(|r| r.is_ok())
                    .unwrap_or(false);

                    if write_ok {
                        let _ = crate::db::update_thumbnail_path(&pool, image_id, &thumb_path_str).await;
                        // First Emit: signal the UI that the preview is ready
                        use tauri::Emitter;
                        let _ = app.emit(
                            "image_updated",
                            crate::models::ImageUpdatedPayload { image_id },
                        );
                    }

                    decoded.push(x);
                }
                Ok(Err((sem_entry, sub_entry, err_msg))) => {
                    eprintln!("[pipeline] decode failed: {err_msg}");
                    if let Some((sem_qid, sem_attempts)) = sem_entry {
                        let _ = crate::db::mark_failed(&pool, sem_qid, sem_attempts, &err_msg).await;
                    }
                    if let Some((sub_qid, sub_attempts)) = sub_entry {
                        let _ = crate::db::mark_failed(&pool, sub_qid, sub_attempts, &err_msg).await;
                    }
                }
                Err(e) => eprintln!("[pipeline] decode task panicked: {e}"),
            }
        }
```

- [ ] **Step 2: Remove old thumbnail generation code from Stage 2**

In `src-tauri/src/pipeline/mod.rs`, locate the end of Phase B in Stage 2 (around line 331) and remove the old thumbnail generation code:

```rust
            // Thumbnail from same buffer — unchanged from original
            let thumb_path = crate::thumbnail::thumbnail_path_for(&data_dir, image_id);
            let thumb_path_str = thumb_path.to_string_lossy().to_string();
            let d_thumb = d.clone();
            let write_ok = tokio::task::spawn_blocking(move || {
                crate::thumbnail::write_thumbnail_from_image(d_thumb.full.as_ref(), &thumb_path)
            })
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
            if write_ok {
                let _ = crate::db::update_thumbnail_path(&pool, image_id, &thumb_path_str).await;
            }
```
**Leave the `image_updated` emit in place at the end of Stage 2** (around line 344), as this serves as the Second Emit signaling that ML analysis is complete.

- [ ] **Step 3: Test compilation**

Run: `cargo check` inside `src-tauri/`
Expected: Passes without errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/pipeline/mod.rs
git commit -m "feat: generate thumbnail immediately after image decode (TT-9)"
```