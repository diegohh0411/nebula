use anyhow::Result;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use crate::indexer::Indexer;
use crate::models::{DebouncedEvent, DebouncedEventKind};

pub struct FolderWatcher {
    inner: RecommendedWatcher,
}

impl FolderWatcher {
    pub fn new(event_tx: mpsc::UnboundedSender<Event>) -> Result<Self> {
        let inner = notify::recommended_watcher(move |res: notify::Result<Event>| {
            match res {
                Ok(event) => {
                    let _ = event_tx.send(event);
                }
                Err(e) => eprintln!("file watcher backend error: {e}"),
            }
        })?;
        Ok(Self { inner })
    }

    pub fn watch(&mut self, path: PathBuf, _folder_id: i64) -> Result<()> {
        self.inner.watch(&path, RecursiveMode::Recursive)?;
        Ok(())
    }

    pub fn unwatch(&mut self, path: &Path) -> Result<()> {
        self.inner.unwatch(path)?;
        Ok(())
    }
}

pub async fn run_debounce_loop(
    mut rx: mpsc::UnboundedReceiver<Event>,
    indexer: Arc<Indexer>,
) {
    let mut debounce_map: HashMap<PathBuf, (DebouncedEventKind, Instant)> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_millis(200));

    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                for path in event.paths {
                    let kind = match event.kind {
                        EventKind::Create(_) => DebouncedEventKind::Create,
                        EventKind::Modify(_) => DebouncedEventKind::Modify,
                        EventKind::Remove(_) => DebouncedEventKind::Remove,
                        _ => continue,
                    };
                    coalesce(&mut debounce_map, path, kind);
                }
            }
            _ = interval.tick() => {
                let now = Instant::now();
                let expired: Vec<DebouncedEvent> = debounce_map
                    .iter()
                    .filter(|(_, (_, instant))| now.duration_since(*instant) > Duration::from_millis(500))
                    .map(|(path, (kind, _))| DebouncedEvent {
                        path: path.clone(),
                        kind: kind.clone(),
                    })
                    .collect();

                for event in &expired {
                    debounce_map.remove(&event.path);
                }

                if !expired.is_empty() {
                    indexer.handle_event_batch(expired).await;
                }
            }
        }
    }
}

fn coalesce(
    map: &mut HashMap<PathBuf, (DebouncedEventKind, Instant)>,
    path: PathBuf,
    incoming: DebouncedEventKind,
) {
    let now = Instant::now();
    match map.get(&path) {
        Some((DebouncedEventKind::Create, _)) => match incoming {
            DebouncedEventKind::Remove => {
                map.remove(&path);
            }
            DebouncedEventKind::Modify | DebouncedEventKind::Create => {
                map.insert(path, (DebouncedEventKind::Create, now));
            }
        },
        Some((DebouncedEventKind::Modify, _)) => match incoming {
            DebouncedEventKind::Remove => {
                map.insert(path, (DebouncedEventKind::Remove, now));
            }
            DebouncedEventKind::Create => {
                map.insert(path, (DebouncedEventKind::Modify, now));
            }
            DebouncedEventKind::Modify => {
                map.insert(path, (DebouncedEventKind::Modify, now));
            }
        },
        Some((DebouncedEventKind::Remove, _)) => match incoming {
            DebouncedEventKind::Create => {
                map.insert(path, (DebouncedEventKind::Modify, now));
            }
            DebouncedEventKind::Modify => {
                map.insert(path, (DebouncedEventKind::Modify, now));
            }
            DebouncedEventKind::Remove => {
                map.insert(path, (DebouncedEventKind::Remove, now));
            }
        },
        None => {
            map.insert(path, (incoming, now));
        }
    }
}
