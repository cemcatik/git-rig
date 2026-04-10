use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

// ---------------------------------------------------------------------------
// Fetch deduplication
// ---------------------------------------------------------------------------

/// Ensures each unique (source_path, remote) pair is fetched exactly once,
/// even when multiple worker threads request it concurrently.
pub struct FetchCache {
    state: Mutex<HashMap<(PathBuf, String), FetchState>>,
    done: Condvar,
}

enum FetchState {
    InProgress,
    Done(Result<(), String>),
}

impl FetchCache {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HashMap::new()),
            done: Condvar::new(),
        }
    }

    /// Fetch the given source+remote exactly once. Concurrent callers with the
    /// same key block until the first caller's fetch completes, then share the result.
    pub fn fetch_once(
        &self,
        source: &Path,
        remote: &str,
        do_fetch: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let key = (source.to_path_buf(), remote.to_string());
        let mut cache = self.state.lock().unwrap();

        loop {
            match cache.get(&key) {
                Some(FetchState::Done(result)) => return result.clone(),
                Some(FetchState::InProgress) => {
                    // Wait for the fetching thread to signal completion
                    cache = self.done.wait(cache).unwrap();
                }
                None => {
                    // We'll do the fetch
                    cache.insert(key.clone(), FetchState::InProgress);
                    drop(cache);

                    let result = do_fetch();

                    let mut cache = self.state.lock().unwrap();
                    cache.insert(key, FetchState::Done(result.clone()));
                    self.done.notify_all();
                    return result;
                }
            }
        }
    }
}

/// Progress handle passed to per-repo operation closures.
pub struct RepoProgress<'a> {
    bar: &'a ProgressBar,
    is_tty: bool,
}

impl RepoProgress<'_> {
    /// Update the spinner's status message (e.g., "fetching...", "rebasing...").
    pub fn set_status(&self, msg: &str) {
        if self.is_tty {
            self.bar.set_message(msg.to_string());
        }
    }
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template(" {spinner:.cyan} {prefix:<20!.bold} {wide_msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "·"])
}

fn success_style() -> ProgressStyle {
    ProgressStyle::with_template(" ✓ {prefix:<20!.bold} {wide_msg:.green}").unwrap()
}

fn failure_style() -> ProgressStyle {
    ProgressStyle::with_template(" ✗ {prefix:<20!.bold} {wide_msg:.red}").unwrap()
}

/// Run `op` for each item in parallel with up to `jobs` workers.
///
/// `names` must already be sorted by the caller (alphabetical order).
/// Returns results in the same order as the input slice.
///
/// The `cancel` flag can be set by the operation closure to signal that
/// remaining work should be skipped (used for --fail-fast).
pub fn run_parallel<T, F>(
    names: &[String],
    jobs: usize,
    cancel: &AtomicBool,
    op: F,
) -> Vec<(String, Result<T, String>)>
where
    T: Send,
    F: Fn(usize, &RepoProgress) -> Result<T, String> + Sync,
{
    let mp = MultiProgress::new();
    let is_tty = !mp.is_hidden();

    // Create spinner bars in sorted order — all visible before work starts
    let style = spinner_style();
    let bars: Vec<ProgressBar> = names
        .iter()
        .map(|name| {
            let pb = mp.add(ProgressBar::new_spinner());
            pb.set_style(style.clone());
            pb.set_prefix(name.clone());
            pb.set_message("queued");
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        })
        .collect();

    type ResultSlot<T> = std::sync::Mutex<Option<(String, Result<T, String>)>>;

    // Pre-allocate result slots — each worker writes to its own index (no contention)
    let results: Vec<ResultSlot<T>> = names.iter().map(|_| std::sync::Mutex::new(None)).collect();

    // Lock-free work queue: each worker atomically grabs the next index
    let next_index = AtomicUsize::new(0);

    thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    // Check for cancellation (--fail-fast)
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }

                    let idx = next_index.fetch_add(1, Ordering::Relaxed);
                    if idx >= names.len() {
                        break;
                    }

                    let progress = RepoProgress {
                        bar: &bars[idx],
                        is_tty,
                    };

                    let result = op(idx, &progress);

                    // Update spinner to final state
                    match &result {
                        Ok(_) => {
                            bars[idx].set_style(success_style());
                            bars[idx].finish_with_message("done");
                        }
                        Err(e) => {
                            bars[idx].set_style(failure_style());
                            bars[idx].abandon_with_message(e.clone());
                        }
                    }

                    if !is_tty {
                        match &result {
                            Ok(_) => eprintln!("  ✓ {}", names[idx]),
                            Err(e) => eprintln!("  ✗ {} — {e}", names[idx]),
                        }
                    }

                    *results[idx].lock().unwrap() = Some((names[idx].clone(), result));
                }
            });
        }
    });

    // Mark cancelled (unstarted) bars so they don't remain in "queued" state
    for (i, slot) in results.iter().enumerate() {
        if slot.lock().unwrap().is_none() {
            bars[i].set_style(failure_style());
            bars[i].abandon_with_message("cancelled".to_string());
        }
    }

    // Collect in order (all slots should be filled, or skipped by cancellation)
    results
        .into_iter()
        .enumerate()
        .filter_map(|(i, slot)| {
            slot.into_inner()
                .unwrap()
                .or_else(|| Some((names[i].clone(), Err("cancelled".to_string()))))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn names(n: usize) -> Vec<String> {
        (0..n).map(|i| format!("item-{i}")).collect()
    }

    #[test]
    fn run_parallel_returns_results_in_order() {
        let items = names(5);
        let cancel = AtomicBool::new(false);

        let results = run_parallel(&items, 3, &cancel, |idx, _progress| Ok(idx));

        assert_eq!(results.len(), 5);
        for (i, (name, result)) in results.iter().enumerate() {
            assert_eq!(name, &format!("item-{i}"));
            assert_eq!(result.as_ref().unwrap(), &i);
        }
    }

    #[test]
    fn run_parallel_empty_input() {
        let items: Vec<String> = vec![];
        let cancel = AtomicBool::new(false);

        let results = run_parallel::<(), _>(&items, 4, &cancel, |_idx, _progress| Ok(()));

        assert!(results.is_empty());
    }

    #[test]
    fn run_parallel_more_jobs_than_items() {
        let items = names(2);
        let cancel = AtomicBool::new(false);

        let results = run_parallel(&items, 10, &cancel, |idx, _progress| Ok(idx));

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].1.as_ref().unwrap(), &0);
        assert_eq!(results[1].1.as_ref().unwrap(), &1);
    }

    #[test]
    fn run_parallel_cancellation_prevents_work() {
        let items = names(10);
        let cancel = AtomicBool::new(false);
        let completed = AtomicUsize::new(0);

        let results = run_parallel(&items, 1, &cancel, |idx, _progress| {
            // Cancel after the first item completes
            if idx == 0 {
                cancel.store(true, Ordering::Relaxed);
            }
            completed.fetch_add(1, Ordering::Relaxed);
            Ok(idx)
        });

        assert_eq!(results.len(), 10);
        // First item should succeed
        assert!(results[0].1.is_ok());
        // With 1 worker, cancellation should prevent most remaining items
        let done_count = completed.load(Ordering::Relaxed);
        assert!(
            done_count < 10,
            "Expected cancellation to prevent some items, but all {done_count} completed"
        );
        // Cancelled items should have Err("cancelled")
        let cancelled_count = results.iter().filter(|(_, r)| r.is_err()).count();
        assert!(cancelled_count > 0, "Expected some cancelled items");
    }

    #[test]
    fn run_parallel_collects_errors() {
        let items = names(3);
        let cancel = AtomicBool::new(false);

        let results = run_parallel(&items, 2, &cancel, |idx, _progress| {
            if idx == 1 {
                Err("something failed".to_string())
            } else {
                Ok(idx)
            }
        });

        assert_eq!(results.len(), 3);
        assert!(results[0].1.is_ok());
        assert_eq!(results[1].1.as_ref().unwrap_err(), "something failed");
        assert!(results[2].1.is_ok());
    }

    #[test]
    fn fetch_cache_deduplicates_same_key() {
        let cache = FetchCache::new();
        let call_count = AtomicUsize::new(0);

        // First call should execute the closure
        let r1 = cache.fetch_once(Path::new("/repo"), "origin", || {
            call_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        });
        assert!(r1.is_ok());
        assert_eq!(call_count.load(Ordering::Relaxed), 1);

        // Second call with same key should return cached result
        let r2 = cache.fetch_once(Path::new("/repo"), "origin", || {
            call_count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        });
        assert!(r2.is_ok());
        assert_eq!(call_count.load(Ordering::Relaxed), 1); // Still 1 — cached
    }

    #[test]
    fn fetch_cache_different_remotes_are_independent() {
        let cache = FetchCache::new();
        let call_count = AtomicUsize::new(0);

        cache
            .fetch_once(Path::new("/repo"), "origin", || {
                call_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .unwrap();

        cache
            .fetch_once(Path::new("/repo"), "upstream", || {
                call_count.fetch_add(1, Ordering::Relaxed);
                Ok(())
            })
            .unwrap();

        // Both should have executed — different remotes
        assert_eq!(call_count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn fetch_cache_propagates_errors() {
        let cache = FetchCache::new();

        let r1 = cache.fetch_once(Path::new("/repo"), "origin", || {
            Err("network error".to_string())
        });
        assert_eq!(r1.unwrap_err(), "network error");

        // Second call should return the cached error, not retry
        let r2 = cache.fetch_once(Path::new("/repo"), "origin", || Ok(()));
        assert_eq!(r2.unwrap_err(), "network error");
    }

    #[test]
    fn fetch_cache_concurrent_same_key() {
        let cache = FetchCache::new();
        let call_count = AtomicUsize::new(0);

        // Spawn multiple threads all requesting the same key
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    cache
                        .fetch_once(Path::new("/repo"), "origin", || {
                            // Simulate slow fetch
                            thread::sleep(Duration::from_millis(50));
                            call_count.fetch_add(1, Ordering::Relaxed);
                            Ok(())
                        })
                        .unwrap();
                });
            }
        });

        // Only one thread should have actually fetched
        assert_eq!(call_count.load(Ordering::Relaxed), 1);
    }
}
